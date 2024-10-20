use chrono::Utc;
use std::pin::pin;

use futures::TryStreamExt;
use k8s_openapi::api::core::v1::{Node, NodeCondition, Taint};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::api::PostParams;
use kube::runtime::reflector::Lookup;
use kube::{
    api::Api,
    client::Client,
    runtime::{watcher, WatchStreamExt},
};
use regex::Regex;

#[derive(Debug)]
pub struct Condition {
    pub type_: Regex,
    pub status: Regex,
}

pub struct Configuration {
    pub conditions: Vec<Condition>,
    pub taint: Taint,
}

pub struct Reconciler {
    node_client: Api<Node>,
    matchers: Vec<Configuration>,
}

impl Reconciler {
    pub fn new(client: Client, matchers: Vec<Configuration>) -> Reconciler {
        Reconciler {
            node_client: Api::all(client),
            matchers,
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

        let status = node.status.as_ref().expect("node should have a status");
        let conditions = status.conditions.as_ref();

        // If a node has no conditions, then we cannot determine whether it's eligible.
        // I'm unsure if this can happen in practice.
        if conditions.is_none() {
            return;
        }

        let mut taints_to_add: Vec<Taint> = vec![];

        let mut node = node.clone();

        let mut spec = node.spec.expect("node should have a spec");
        // We deliberately unwrap_or_default to gracefully handle nodes with no taints.
        let mut taints = spec.taints.unwrap_or_default();

        for matcher in &self.matchers {
            if !self.is_node_eligible(
                node_name.as_ref(),
                conditions.unwrap(),
                matcher.conditions.as_ref(),
            ) {
                continue;
            }

            let taint = &matcher.taint;

            // Don't attempt to add the taint if the node already has it.
            if self.node_has_taint(&taints, taint) {
                tracing::info!(
                    node = node_name.as_ref(),
                    taint = self.taint_to_string(taint),
                    "Node matches conditions but already has taint"
                );
                continue;
            }

            let mut taint_to_add = taint.clone();

            // Only set time_added for NoExecute taints.
            // See https://kubernetes.io/docs/reference/generated/kubernetes-api/v1.27/#taint-v1-core.
            if &taint_to_add.effect == "NoExecute" {
                let time_added = Time(Utc::now());
                taint_to_add.time_added = Some(time_added)
            }

            taints_to_add.push(taint_to_add)
        }

        // Return immediately if we have no taints to add to the node.
        if taints_to_add.is_empty() {
            return;
        }

        let taints_string = format!("{:?}", taints_to_add);
        taints.append(taints_to_add.as_mut());
        spec.taints = Some(taints);
        node.spec = Some(spec);

        let params = &PostParams {
            dry_run: false,
            field_manager: Some(String::from("tainter")),
        };
        tracing::info!(
            node = node_name.as_ref(),
            taints = taints_string,
            "Adding taints to node"
        );
        if let Err(error) = self
            .node_client
            .replace(node_name.as_ref(), params, &node)
            .await
        {
            let error_string = error.to_string();
            // Conflict errors can happen when another process (perhaps another Tainter process?)
            // modifies a node before this Tainter process can execute its update request.
            // When this happens, Tainter will receive an HTTP 409 Conflict response.
            // The fact that the node was modified means that Tainter will pick up another
            // modification event and re-evaluate the node, essentially providing automatic retry.
            if self.is_conflict_error(error_string.as_str()) {
                tracing::info!(
                    error = error_string,
                    node = node_name.as_ref(),
                    taints = taints_string,
                    "Received conflict error when trying to add taints to node"
                )
            } else {
                tracing::error!(
                    error = error_string,
                    node = node_name.as_ref(),
                    taints = taints_string,
                    "Error adding taints to node"
                )
            }
        } else {
            tracing::info!(
                node = node_name.as_ref(),
                taints = taints_string,
                "Successfully added taints to node"
            )
        }
    }

    fn is_conflict_error(&self, error_string: &str) -> bool {
        error_string.contains("the object has been modified; please apply your changes to the latest version and try again")
    }

    fn node_has_taint(&self, haystack: &Vec<Taint>, needle: &Taint) -> bool {
        for taint in haystack {
            if self.identical_taints(taint, needle) {
                return true;
            }
        }

        false
    }

    // Identical taints are defined as ones that have the same key and effect.
    // See this error from the Kubernetes API:
    //
    // Node "XYZ" is invalid: metadata.taints[1]: Duplicate value: core.Taint{Key:"kubernetes.azure.com/scalesetpriority", Value:"premium", Effect:"NoSchedule", TimeAdded:<nil>}: taints must be unique by key and effect pair
    //
    fn identical_taints(&self, this: &Taint, that: &Taint) -> bool {
        this.key == that.key && this.effect == that.effect
    }

    fn is_node_eligible(
        &self,
        node_name: &str,
        have: &Vec<NodeCondition>,
        want: &Vec<Condition>,
    ) -> bool {
        'search: for desired_condition in want {
            for node_condition in have {
                if self.conditions_match(desired_condition, node_condition) {
                    tracing::info!(
                        node = node_name,
                        node_condition = format!("{:?}", node_condition).as_str(),
                        condition = format!("{:?}", desired_condition).as_str(),
                        "Node matches condition",
                    );
                    continue 'search;
                }
            }

            // If we can't find a match for a single condition, the node is not eligible.
            return false;
        }

        true
    }

    fn conditions_match(&self, this: &Condition, that: &NodeCondition) -> bool {
        let statuses_match = this.status.is_match(that.status.as_str());
        let types_match = this.type_.is_match(that.type_.as_str());

        statuses_match && types_match
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

    async fn setup(
        list_response_file: &str,
        matchers: Vec<Configuration>,
    ) -> Handle<Request<Body>, Response<Body>> {
        // https://kube.rs/controllers/testing/#example.
        let (mock_service, mut handle) = tower_test::mock::pair::<Request<Body>, Response<Body>>();

        let client = Client::new(mock_service, "default");

        let reconciler = Reconciler::new(client, matchers);

        tokio::spawn(async move {
            reconciler.start().await;
        });

        let (request, response) = handle.next_request().await.expect("list nodes not called");
        assert_eq!(request.method(), http::Method::GET);
        assert_eq!(request.uri().to_string(), "/api/v1/nodes?&limit=500");

        let node_list_response_body = get_test_file(list_response_file);

        response.send_response(
            Response::builder()
                .body(Body::from(node_list_response_body.into_bytes()))
                .unwrap(),
        );

        handle
    }

    #[tokio::test]
    #[traced_test]
    async fn test_start_checks_conditions_with_regex_and_adds_taints() {
        let matchers = vec![
            Configuration {
                taint: Taint {
                    effect: "NoExecute".to_string(),
                    key: "pressure".to_string(),
                    time_added: None,
                    value: Some("memory".to_string()),
                },
                conditions: vec![Condition {
                    type_: Regex::new("OutOfMemory").unwrap(),
                    status: Regex::new("True").unwrap(),
                }],
            },
            Configuration {
                taint: Taint {
                    effect: "NoSchedule".to_string(),
                    key: "network-partition".to_string(),
                    time_added: None,
                    value: None,
                },
                conditions: vec![
                    Condition {
                        type_: Regex::new("NetworkInterfaceCard").unwrap(),
                        status: Regex::new("Kaput|Ruined").unwrap(),
                    },
                    Condition {
                        type_: Regex::new("PrivateLink").unwrap(),
                        status: Regex::new("Severed").unwrap(),
                    },
                ],
            },
        ];
        let mut handle = setup("list-nodes-multiple-eligible-regex.json", matchers).await;

        let (request, response) = handle
            .next_request()
            .await
            .expect("PUT node not called for aks-artemis1-41950716-vmss000082");
        assert_eq!(request.method(), http::Method::PUT);
        assert_eq!(
            request.uri().to_string(),
            "/api/v1/nodes/aks-artemis1-41950716-vmss000082?&fieldManager=tainter"
        );
        let node = node_from_body(request).await;
        let taints = node.spec.unwrap().taints.unwrap();
        assert_eq!(taints.len(), 2);
        let taint = taints.get(1).unwrap();
        let expected = Taint {
            effect: "NoExecute".to_string(),
            key: "pressure".to_string(),
            time_added: Some(Time(Utc::now())),
            value: Some("memory".to_string()),
        };
        assert_eq!(taint.effect, expected.effect);
        assert_eq!(taint.key, expected.key);
        assert_eq!(taint.value, expected.value);
        assert!(within_duration(
            taint.time_added.as_ref().unwrap().0,
            expected.time_added.as_ref().unwrap().0,
            chrono::Duration::seconds(5),
        ));

        response.send_response(
            Response::builder()
                .body(Body::from(
                    get_test_file("node-put-success.json").into_bytes(),
                ))
                .unwrap(),
        );

        let (request, response) = handle
            .next_request()
            .await
            .expect("PUT node not called for aks-poseidon1-41950716-vmss000082");
        assert_eq!(request.method(), http::Method::PUT);
        assert_eq!(
            request.uri().to_string(),
            "/api/v1/nodes/aks-poseidon1-41950716-vmss000082?&fieldManager=tainter"
        );
        let node = node_from_body(request).await;
        let taints = node.spec.unwrap().taints.unwrap();
        assert_eq!(taints.len(), 3);
        let taint = taints.get(1).unwrap();
        let expected = Taint {
            effect: "NoExecute".to_string(),
            key: "pressure".to_string(),
            time_added: Some(Time(Utc::now())),
            value: Some("memory".to_string()),
        };
        assert_eq!(taint.effect, expected.effect);
        assert_eq!(taint.key, expected.key);
        assert_eq!(taint.value, expected.value);
        assert!(within_duration(
            taint.time_added.as_ref().unwrap().0,
            expected.time_added.as_ref().unwrap().0,
            chrono::Duration::seconds(5),
        ));

        let taint = taints.get(2).unwrap();
        let expected = Taint {
            effect: "NoSchedule".to_string(),
            key: "network-partition".to_string(),
            time_added: None,
            value: None,
        };
        assert_eq!(taint.effect, expected.effect);
        assert_eq!(taint.key, expected.key);
        assert_eq!(taint.value, expected.value);
        assert_eq!(taint.time_added, expected.time_added);

        response.send_response(
            Response::builder()
                .body(Body::from(
                    get_test_file("node-put-success.json").into_bytes(),
                ))
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
            r#"Processing node node_name="aks-ares1-41950716-vmss000082""#
        ));
        assert!(logs_contain(
            r#"Processing node node_name="aks-artemis1-41950716-vmss000082""#
        ));
        assert!(logs_contain(
            r#"Adding taints to node node="aks-artemis1-41950716-vmss000082" taints="[Taint { effect: \"NoExecute\", key: \"pressure\""#
        ));
        assert!(logs_contain(
            r#"Successfully added taints to node node="aks-artemis1-41950716-vmss000082" taints="[Taint { effect: \"NoExecute\", key: \"pressure\""#
        ));
        assert!(logs_contain(
            r#"Node matches condition node="aks-artemis1-41950716-vmss000082" node_condition="NodeCondition { last_heartbeat_time: Some(Time(2024-05-12T11:21:10Z)), last_transition_time: Some(Time(2024-05-07T08:32:09Z)), message: Some(\"The VM has no surplus memory\"), reason: Some(\"NoSurplusMemory\"), status: \"True\", type_: \"OutOfMemory\" }" condition="Condition { type_: Regex(\"OutOfMemory\"), status: Regex(\"True\") }""#
        ));
        assert!(logs_contain(
            r#"Processing node node_name="aks-athena1-41950716-vmss000082""#
        ));
        assert!(logs_contain(
            r#"Processing node node_name="aks-poseidon1-41950716-vmss000082""#
        ));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_start_processes_node_and_logs_error_if_update_fails() {
        let matchers = vec![Configuration {
            taint: Taint {
                effect: "NoExecute".to_string(),
                key: "event".to_string(),
                time_added: None,
                value: None,
            },
            conditions: vec![Condition {
                type_: Regex::new("VMEventScheduled").unwrap(),
                status: Regex::new("True").unwrap(),
            }],
        }];
        let mut handle = setup("list-nodes-single-eligible.json", matchers).await;

        let (request, response) = handle.next_request().await.expect("PUT node not called");
        assert_eq!(request.method(), http::Method::PUT);
        assert_eq!(
            request.uri().to_string(),
            "/api/v1/nodes/aks-zeus1-41950716-vmss000082?&fieldManager=tainter"
        );

        let node_put_response_body = get_test_file("node-put-invalid-response.json");

        response.send_response(
            Response::builder()
                .body(Body::from(node_put_response_body.into_bytes()))
                .unwrap(),
        );

        let (_, _) = handle.next_request().await.expect("watch nodes not called");

        assert!(logs_contain(
            r#"Error adding taints to node error="Error deserializing response" node="aks-zeus1-41950716-vmss000082" taints="[Taint { effect: \"NoExecute\", key: \"event\""#
        ))
    }

    #[tokio::test]
    #[traced_test]
    async fn test_start_adds_taint_only_if_node_does_not_already_have_it() {
        let matchers = vec![Configuration {
            taint: Taint {
                effect: "NoExecute".to_string(),
                key: "node.kubernetes.io/out-of-service".to_string(),
                time_added: None,
                value: None,
            },
            conditions: vec![Condition {
                type_: Regex::new("Ready").unwrap(),
                status: Regex::new("False|Unknown").unwrap(),
            }],
        }];
        let mut handle = setup("list-nodes-eligible-and-has-taint.json", matchers).await;

        let (request, _) = handle.next_request().await.expect("watch nodes not called");
        assert_eq!(request.method(), http::Method::GET);
        assert_eq!(
            request.uri().to_string(),
            "/api/v1/nodes?&watch=true&timeoutSeconds=290&\
        allowWatchBookmarks=true&resourceVersion=test"
        );

        assert!(logs_contain(
            r#"Node matches conditions but already has taint node="aks-artemis1-41950716-vmss000082" taint="node.kubernetes.io/out-of-service:NoExecute"#
        ))
    }

    #[tokio::test]
    #[traced_test]
    async fn test_start_gracefully_handles_conflict_error() {
        let matchers = vec![Configuration {
            taint: Taint {
                effect: "NoSchedule".to_string(),
                key: "not-ready".to_string(),
                time_added: None,
                value: None,
            },
            conditions: vec![Condition {
                type_: Regex::new("Ready").unwrap(),
                status: Regex::new("False|Unknown").unwrap(),
            }],
        }];
        let mut handle = setup("list-nodes-single-eligible.json", matchers).await;

        let (request, response) = handle.next_request().await.expect("PUT node not called");
        assert_eq!(request.method(), http::Method::PUT);
        assert_eq!(
            request.uri().to_string(),
            "/api/v1/nodes/aks-zeus1-41950716-vmss000082?&fieldManager=tainter"
        );

        let node_put_response_body = get_test_file("node-put-conflict-response.json");

        response.send_response(
            Response::builder()
                .status(409)
                .body(Body::from(node_put_response_body.into_bytes()))
                .unwrap(),
        );

        let (_, _) = handle.next_request().await.expect("watch nodes not called");

        assert!(logs_contain(
            r#"Received conflict error when trying to add taints to node error="ApiError: Operation cannot be fulfilled on nodes \"aks-zeus1-41950716-vmss000082\": the object has been modified; please apply your changes to the latest version and try again: Conflict (ErrorResponse { status: \"Failure\", message: \"Operation cannot be fulfilled on nodes \\\"aks-zeus1-41950716-vmss000082\\\": the object has been modified; please apply your changes to the latest version and try again\", reason: \"Conflict\", code: 409 })" node="aks-zeus1-41950716-vmss000082" taints="[Taint { effect: \"NoSchedule\", key: \"not-ready\", time_added: None, value: None }]"#
        ));
        assert!(!logs_contain("Error adding taint to node"))
    }

    fn get_file_content(path: PathBuf) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn get_test_file(name: &str) -> String {
        get_file_content(
            Path::new(".")
                .join("src")
                .join("reconciler")
                .join("testfiles")
                .join(name),
        )
    }

    async fn node_from_body(request: Request<Body>) -> Node {
        let bytes = request.into_body().collect_bytes().await.unwrap();
        let body_string = String::from_utf8(bytes.into_iter().collect()).unwrap();
        let node: Node = serde_json::from_str(body_string.as_str()).unwrap();

        node
    }

    // assert that time is within plus minus duration of target.
    fn within_duration(
        time: chrono::DateTime<Utc>,
        target: chrono::DateTime<Utc>,
        duration: chrono::Duration,
    ) -> bool {
        let from = target - duration;
        let to = target + duration;

        let within_duration = from <= time && time <= to;
        within_duration
    }

    #[tokio::test]
    #[traced_test]
    async fn test_start_logs_error_if_list_nodes_fails() {
        let (mock_service, mut handle) = tower_test::mock::pair::<Request<Body>, Response<Body>>();

        let client = Client::new(mock_service, "default");

        let matchers = vec![Configuration {
            taint: Taint {
                effect: "NoExecute".to_string(),
                key: "bird".to_string(),
                time_added: None,
                value: Some("flamingo".to_string()),
            },
            conditions: vec![Condition {
                type_: Regex::new("animal").unwrap(),
                status: Regex::new("(?i)flamingo").unwrap(),
            }],
        }];
        let reconciler = Reconciler::new(client, matchers);

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
