use std::pin::pin;

use futures::TryStreamExt;
use k8s_openapi::api::core::v1::{Node, Taint};
use kube::api::PostParams;
use kube::runtime::reflector::Lookup;
use kube::{
    api::Api,
    client::Client,
    runtime::{watcher, WatchStreamExt},
};

pub struct Reconciler {
    node_client: Api<Node>,
}

impl Reconciler {
    pub fn new(client: Client) -> Reconciler {
        Reconciler {
            node_client: Api::all(client),
        }
    }

    pub async fn start(&self) {
        // https://github.com/kube-rs/kube/blob/dac48d96a7b72a88fdf60857e751b122b79a3cc4/examples/node_watcher.rs.
        let wc = watcher::Config::default();
        let obs = watcher(self.node_client.clone(), wc)
            .default_backoff()
            .applied_objects();
        let mut obs = pin!(obs);

        loop {
            let result = obs.try_next().await;

            match result {
                Ok(node) => {
                    match node {
                        Some(node) => self.process_node(node).await,
                        None => {
                            // I'm not sure if this can happen in practice.
                            tracing::info!("Node is none")
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(error = error.to_string())
                }
            }
        }
    }

    async fn process_node(&self, node: Node) {
        let node_name = node.name().expect("node should have a name");
        tracing::info!(node_name = node_name.as_ref(), "Processing node");

        // TODO: only apply out-of-service taint if node doesn't already have it.
        // TODO: only apply out-of-service taint if node fulfils criteria.

        let now = chrono::offset::Utc::now();

        let taint = Taint {
            effect: "NoExecute".to_string(),
            key: "node.kubernetes.io/out-of-service".to_string(),
            time_added: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(now)),
            value: None,
        };
        let taint_string = self.taint_to_string(&taint);

        let mut node = node.clone();
        let mut spec = node.spec.expect("node should have a spec");
        // TODO what if node has no taints?
        let mut taints = spec.taints.unwrap_or_default();
        taints.push(taint);
        spec.taints = Some(taints);
        node.spec = Some(spec);

        let params = &PostParams {
            dry_run: false,
            // TODO: perhaps field_manager should be configurable.
            field_manager: Some(String::from("tainter")),
        };
        // TODO: handle 409 Conflict HTTP error.
        // TODO: handle duplicate taint error.
        if let Err(error) = self
            .node_client
            .replace(node_name.as_ref(), params, &node)
            .await
        {
            tracing::error!(
                error = error.to_string(),
                node = node_name.as_ref(),
                "Error adding taint to node"
            )
        } else {
            tracing::info!(
                node = node_name.as_ref(),
                taint = taint_string,
                "Successfully added taint to node"
            )
        }
    }

    fn taint_to_string(&self, taint: &Taint) -> String {
        let value = match taint.value.as_ref() {
            None => String::new(),
            Some(value) => format!("={}", value),
        };
        let time_added = match taint.time_added.as_ref() {
            None => String::new(),
            Some(time_added) => format!("/{}", time_added.0),
        };
        format!("{}{value}:{}{time_added}", taint.key, taint.effect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use http::{Request, Response};
    use k8s_openapi::serde_json;
    use kube::client::Body;
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};
    use std::{fs, io};
    use tower_test::mock::Handle;
    use tracing_test::traced_test;

    async fn setup(list_response_file: &str) -> Handle<Request<Body>, Response<Body>> {
        // https://kube.rs/controllers/testing/#example.
        let (mock_service, mut handle) = tower_test::mock::pair::<Request<Body>, Response<Body>>();

        let client = Client::new(mock_service, "default");

        let reconciler = Reconciler::new(client);

        tokio::spawn(async move {
            reconciler.start().await;
        });

        let (request, response) = handle.next_request().await.expect("list nodes not called");
        assert_eq!(request.method(), http::Method::GET);
        assert_eq!(request.uri().to_string(), "/api/v1/nodes?&limit=500");

        let node_list_response_body = get_file_content(
            Path::new(".")
                .join("src")
                .join("reconciler")
                .join("testfiles")
                .join(list_response_file),
        );

        response.send_response(
            Response::builder()
                .body(Body::from(node_list_response_body.into_bytes()))
                .unwrap(),
        );

        handle
    }

    #[tokio::test]
    #[traced_test]
    async fn test_start_processes_node_and_logs_error_if_update_fails() {
        let mut handle = setup("list-nodes.json").await;

        let (request, response) = handle.next_request().await.expect("PUT node not called");
        assert_eq!(request.method(), http::Method::PUT);
        assert_eq!(
            request.uri().to_string(),
            "/api/v1/nodes/aks-zeus1-41950716-vmss000082?&fieldManager=tainter"
        );

        let node_put_response_body = get_file_content(
            Path::new(".")
                .join("src")
                .join("reconciler")
                .join("testfiles")
                .join("node-put-invalid-response.json"),
        );

        response.send_response(
            Response::builder()
                .body(Body::from(node_put_response_body.into_bytes()))
                .unwrap(),
        );

        let (_, _) = handle.next_request().await.expect("watch nodes not called");

        assert!(logs_contain(
            r#"Error adding taint to node error="Error deserializing response" node="aks-zeus1-41950716-vmss000082""#
        ))
    }

    #[tokio::test]
    #[traced_test]
    async fn test_start_processes_node_and_adds_taint() {
        let mut handle = setup("list-nodes.json").await;

        let (request, response) = handle.next_request().await.expect("PUT node not called");
        assert_eq!(request.method(), http::Method::PUT);
        assert_eq!(
            request.uri().to_string(),
            "/api/v1/nodes/aks-zeus1-41950716-vmss000082?&fieldManager=tainter"
        );
        let bytes = request.into_body().collect_bytes().await.unwrap();
        let body_string = String::from_utf8(bytes.into_iter().collect()).unwrap();
        let node: Node = serde_json::from_str(body_string.as_str()).unwrap();
        let taints = node.spec.unwrap().taints.unwrap();
        assert_eq!(taints.len(), 2);
        let taint = taints.get(1).unwrap();
        assert_out_of_service_taint(taint);

        let node_put_response_body = get_file_content(
            Path::new(".")
                .join("src")
                .join("reconciler")
                .join("testfiles")
                .join("node-put-success.json"),
        );

        response.send_response(
            Response::builder()
                .body(Body::from(node_put_response_body.into_bytes()))
                .unwrap(),
        );

        let (request, _) = handle.next_request().await.expect("watch nodes not called");
        assert_eq!(request.method(), http::Method::GET);
        assert_eq!(
            request.uri().to_string(),
            "/api/v1/nodes?&watch=true&timeoutSeconds=290&\
        allowWatchBookmarks=true&resourceVersion=test"
        );

        assert!(logs_contain(
            r#"Processing node node_name="aks-zeus1-41950716-vmss000082""#
        ));
        assert!(logs_contain(
            r#"Successfully added taint to node node="aks-zeus1-41950716-vmss000082" taint="node.kubernetes.io/out-of-service:NoExecute/"#
        ));
    }

    fn get_file_content(path: PathBuf) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn assert_out_of_service_taint(taint: &Taint) {
        assert_eq!(taint.effect, "NoExecute");
        assert_eq!(taint.key, "node.kubernetes.io/out-of-service");
        assert_eq!(taint.value, None);
        assert!(within_duration(
            taint.time_added.as_ref().unwrap().0,
            chrono::Duration::seconds(5)
        ));
    }

    fn within_duration(time: chrono::DateTime<Utc>, duration: chrono::Duration) -> bool {
        let now = Utc::now();
        let from = now - duration;
        let to = now + duration;

        let within_duration = from <= time && time <= to;
        within_duration
    }

    #[tokio::test]
    #[traced_test]
    async fn test_start_logs_error_if_list_nodes_fails() {
        let (mock_service, mut handle) = tower_test::mock::pair::<Request<Body>, Response<Body>>();

        let client = Client::new(mock_service, "default");

        let reconciler = Reconciler::new(client);

        tokio::spawn(async move {
            reconciler.start().await;
        });

        let (request, response) = handle
            .next_request()
            .await
            .expect("GET nodes not called first time");
        assert_eq!(request.method(), http::Method::GET);
        assert_eq!(request.uri().to_string(), "/api/v1/nodes?&limit=500");

        let error = io::Error::new(ErrorKind::Interrupted, "some connection error");
        response.send_error(error);

        let (request, _) = handle
            .next_request()
            .await
            .expect("GET nodes not called second time");
        assert_eq!(request.method(), http::Method::GET);
        assert_eq!(request.uri().to_string(), "/api/v1/nodes?&limit=500");

        assert!(logs_contain(
            r#"error="failed to perform initial object list: ServiceError: some connection error""#
        ))
    }
}
