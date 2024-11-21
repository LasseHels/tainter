use chrono::Utc;
use k8s_openapi::api::core::v1::{Node, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::api::ListParams;
use kube::{Api, Client};
use std::collections::HashMap;
use std::ops::Add;

// This test requires a lot of setup (see "make end-to-end-setup") and is relatively expensive to
// run, so exclude it unless it is explicitly included.
#[ignore]
#[tokio::test]
async fn test_end_to_end() {
    assert_tainter_pods_on_different_nodes().await;
    // Add a node condition to make Tainter act.
    add_custom_node_condition().await;

    wait_for_taint_to_be_added().await;

    assert_tainter_added_taint().await;
    // Tainter itself does not tolerate the taint that it adds, and tainting one of the nodes in the
    // cluster may have moved a Tainter pod. We finish the test by re-asserting that Tainter pods
    // still run on different nodes even after a potential pod shuffle.
    assert_tainter_pods_on_different_nodes().await;
}

async fn assert_tainter_pods_on_different_nodes() {
    let pod_client: Api<Pod> = Api::all(Client::try_default().await.unwrap());

    wait_for_tainter_pods_to_be_assigned_nodes(&pod_client).await;

    let tainter_pods = pod_client
        .list(&ListParams::default().labels("app=tainter"))
        .await
        .unwrap();
    let pod_count = tainter_pods.items.len();
    // The deployment spins up two pods, but it is possible for three Tainter pods to exists at
    // once. This happens when a node taint simultaneously causes a Tainter pod to terminate and a
    // new one to spin up.
    assert!(pod_count == 2 || pod_count == 3);

    let mut nodes_with_tainter_pods: HashMap<&str, bool> = HashMap::new();

    for pod in tainter_pods.iter() {
        let pod_node = pod
            .spec
            .as_ref()
            .unwrap()
            .node_name
            .as_ref()
            .unwrap()
            .as_str();
        let node_has_another_tainter_pod = nodes_with_tainter_pods.contains_key(pod_node);
        assert!(
            !node_has_another_tainter_pod,
            "node {} has more than one Tainter pod",
            pod_node
        );
        nodes_with_tainter_pods.insert(pod_node, true);
    }
}

async fn assert_tainter_added_taint() {
    let node_client: Api<Node> = Api::all(Client::try_default().await.unwrap());
    let node = node_client.get("tainter-end-to-end-m02").await.unwrap();
    let taints = node.spec.as_ref().unwrap().taints.as_ref().unwrap();
    assert_eq!(1, taints.len());
    let taint = taints.first().unwrap();
    assert_eq!("taste", taint.key);
    assert_eq!("bland", taint.value.as_ref().unwrap());
    assert_eq!("NoExecute", taint.effect);
    assert_within_duration(
        taint.time_added.as_ref().unwrap().0,
        Time(Utc::now()).0,
        chrono::Duration::seconds(5),
    );
}

async fn add_custom_node_condition() {
    let body = r#"{"status": {"conditions": [{"type": "OutOfChocolate", "status": "True", "reason": "CocoaShortSupply", "message": "We're out of chocolate!"}]}}"#;
    let client = reqwest::Client::new();
    // https://github.com/kubernetes/kubernetes/issues/67455#issuecomment-808531584.
    let res = client
        .patch("http://localhost:8011/api/v1/nodes/tainter-end-to-end-m02/status")
        .body(body)
        .header("Content-Type", "application/merge-patch+json")
        .send()
        .await
        .unwrap();
    assert_eq!(
        http::StatusCode::OK,
        res.status(),
        "Failed to update node condition. Got back status {} with response body {}",
        res.status(),
        res.text().await.unwrap()
    );

    // Verify that the custom condition was added.
    let node_client: Api<Node> = Api::all(Client::try_default().await.unwrap());
    let node = node_client.get("tainter-end-to-end-m02").await.unwrap();
    let conditions = node.status.as_ref().unwrap().conditions.as_ref().unwrap();
    assert_eq!(1, conditions.len());
    let custom_condition = conditions.first().unwrap();
    assert_eq!("OutOfChocolate", custom_condition.type_);
}

// I'm sure there is a better way to implement waiting in wait_for_taint_to_be_added() and
// wait_for_tainter_pods_to_be_assigned_nodes(). I'd like for their loops to not fire off as fast
// as possible, but to instead sleep a bit after each iteration.
async fn wait_for_taint_to_be_added() {
    let deadline = Utc::now().add(chrono::Duration::seconds(120));
    let node_client: Api<Node> = Api::all(Client::try_default().await.unwrap());

    loop {
        let deadline_in_future = Utc::now() < deadline;
        assert!(
            deadline_in_future,
            "timed out waiting for taint to be added to tainter-end-to-end-m02"
        );

        let node = node_client.get("tainter-end-to-end-m02").await.unwrap();
        let taints = node.spec.as_ref().unwrap().taints.as_ref();
        if taints.is_none() {
            continue;
        }
        if taints.unwrap().len() == 1 {
            break;
        }
    }
}

// TODO this should return pods. What if pods change between this function getting pods and the
// caller getting pods?
async fn wait_for_tainter_pods_to_be_assigned_nodes(pod_client: &Api<Pod>) {
    let deadline = Utc::now().add(chrono::Duration::seconds(60));

    loop {
        let deadline_in_future = Utc::now() < deadline;
        assert!(
            deadline_in_future,
            "timed out waiting for all Tainter pods to be assigned nodes"
        );

        let tainter_pods = pod_client
            .list(&ListParams::default().labels("app=tainter"))
            .await
            .unwrap();

        let pods_without_nodes: Vec<&Pod> = tainter_pods.iter().filter(|pod| pod.spec.as_ref().unwrap().node_name.is_none()).collect();

        if pods_without_nodes.is_empty() {
            return
        }
    }
}

// assert that time is within plus minus duration of target.
fn assert_within_duration(
    time: chrono::DateTime<Utc>,
    target: chrono::DateTime<Utc>,
    duration: chrono::Duration,
) {
    let from = target - duration;
    let to = target + duration;

    let within_duration = from <= time && time <= to;
    assert!(
        within_duration,
        "{} is not within {} seconds of {}",
        time.to_string(),
        duration.num_seconds(),
        target.to_string()
    )
}
