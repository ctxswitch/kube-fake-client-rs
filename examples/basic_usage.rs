//! Basic kube::Api operations with both namespaced and cluster-scoped resources
//!
//! Example demonstrating common Kubernetes API operations including:
//! - Creating, reading, updating, and deleting resources
//! - Label and field selectors for filtering
//! - Patching resources
//! - Working with both namespaced and cluster-scoped resources

use k8s_openapi::api::core::v1::{Container, Node, Pod, PodSpec};
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube_fake_client::ClientBuilder;
use serde_json::json;
use std::collections::BTreeMap;

/// Create a pod with the given name, namespace, and optional labels
fn create_pod(name: &str, namespace: &str, labels: Option<BTreeMap<String, String>>) -> Pod {
    let mut pod = Pod::default();
    pod.metadata.name = Some(name.to_string());
    pod.metadata.namespace = Some(namespace.to_string());
    pod.metadata.labels = labels;
    pod
}

/// Create a pod with a container
fn create_pod_with_container(
    name: &str,
    namespace: &str,
    labels: Option<BTreeMap<String, String>>,
    container_name: &str,
    image: &str,
) -> Pod {
    let mut pod = create_pod(name, namespace, labels);
    pod.spec = Some(PodSpec {
        containers: vec![Container {
            name: container_name.to_string(),
            image: Some(image.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });
    pod
}

/// Create a pod scheduled on a specific node
fn create_pod_on_node(
    name: &str,
    namespace: &str,
    node_name: &str,
    container_name: &str,
    image: &str,
) -> Pod {
    let mut pod = create_pod_with_container(name, namespace, None, container_name, image);
    if let Some(spec) = &mut pod.spec {
        spec.node_name = Some(node_name.to_string());
    }
    pod
}

/// Create a node with the given name and zone label
fn create_node(name: &str, zone: &str) -> Node {
    let mut node = Node::default();
    node.metadata.name = Some(name.to_string());
    let mut labels = BTreeMap::new();
    labels.insert("role".to_string(), "worker".to_string());
    labels.insert("zone".to_string(), zone.to_string());
    node.metadata.labels = Some(labels);
    node
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let initial_pod = create_pod("web-server", "default", None);

    let mut labels = BTreeMap::new();
    labels.insert("app".to_string(), "backend".to_string());
    let api_pod = create_pod("api-server", "default", Some(labels));

    let client = ClientBuilder::new()
        .with_objects(vec![initial_pod, api_pod])
        .with_status_subresource::<Pod>()
        .build()
        .await?;

    let pods: Api<Pod> = Api::namespaced(client.clone(), "default");

    let pod_list = pods.list(&ListParams::default()).await?;
    println!("Initial pods: {}", pod_list.items.len());
    for pod in &pod_list.items {
        println!("  - {}", pod.metadata.name.as_ref().unwrap());
    }

    let mut nginx_labels = BTreeMap::new();
    nginx_labels.insert("app".to_string(), "frontend".to_string());
    nginx_labels.insert("env".to_string(), "dev".to_string());
    let new_pod = create_pod_with_container(
        "nginx",
        "default",
        Some(nginx_labels),
        "nginx",
        "nginx:latest",
    );

    let created = pods.create(&PostParams::default(), &new_pod).await?;
    println!("\nCreated pod: {}", created.metadata.name.as_ref().unwrap());

    let retrieved = pods.get("nginx").await?;
    println!(
        "Retrieved pod: {}",
        retrieved.metadata.name.as_ref().unwrap()
    );

    let filtered = pods
        .list(&ListParams::default().labels("app=frontend"))
        .await?;
    println!("\nPods with app=frontend: {}", filtered.items.len());

    // Field selector examples (using pre-registered fields)
    println!("\n=== Field Selector Examples ===\n");

    // Universal field selectors (work for all resource types)
    let by_name = pods
        .list(&ListParams::default().fields("metadata.name=nginx"))
        .await?;
    println!(
        "Pods filtered by metadata.name=nginx: {}",
        by_name.items.len()
    );
    for pod in &by_name.items {
        println!("  - {}", pod.metadata.name.as_ref().unwrap());
    }

    // Create a pod with node name for Pod-specific field selector example
    let scheduled_pod = create_pod_on_node(
        "scheduled-pod",
        "default",
        "worker-node-1",
        "app",
        "app:latest",
    );
    pods.create(&PostParams::default(), &scheduled_pod).await?;

    // Pod-specific pre-registered field selector (spec.nodeName)
    let by_node = pods
        .list(&ListParams::default().fields("spec.nodeName=worker-node-1"))
        .await?;
    println!("\nPods scheduled on worker-node-1: {}", by_node.items.len());
    for pod in &by_node.items {
        println!(
            "  - {} (node: {})",
            pod.metadata.name.as_ref().unwrap(),
            pod.spec
                .as_ref()
                .and_then(|s| s.node_name.as_ref())
                .unwrap_or(&"none".to_string())
        );
    }

    // Update pod status for status.phase field selector example
    let status_patch = json!({
        "status": {
            "phase": "Running"
        }
    });
    pods.patch_status(
        "nginx",
        &PatchParams::default(),
        &Patch::Merge(&status_patch),
    )
    .await?;

    // Pod-specific pre-registered field selector (status.phase)
    let running_pods = pods
        .list(&ListParams::default().fields("status.phase=Running"))
        .await?;
    println!("\nPods in Running phase: {}", running_pods.items.len());

    // Multiple field selectors combined
    let combined = pods
        .list(&ListParams::default().fields("metadata.name=nginx,status.phase=Running"))
        .await?;
    println!(
        "\nPods with name=nginx AND phase=Running: {}",
        combined.items.len()
    );

    let patch = json!({
        "metadata": {
            "labels": {
                "environment": "production"
            }
        }
    });
    let patched = pods
        .patch(
            "nginx",
            &PatchParams::apply("example"),
            &Patch::Merge(&patch),
        )
        .await?;
    println!("\nPatched pod labels: {:?}", patched.metadata.labels);

    let mut updated_pod = patched.clone();
    if let Some(spec) = &mut updated_pod.spec {
        if let Some(container) = spec.containers.first_mut() {
            container.image = Some("nginx:1.21".to_string());
        }
    }
    let updated = pods
        .replace("nginx", &PostParams::default(), &updated_pod)
        .await?;
    println!(
        "Updated image, resource version: {}",
        updated.metadata.resource_version.as_ref().unwrap()
    );

    pods.delete("nginx", &DeleteParams::default()).await?;
    println!("\nDeleted pod: nginx");

    let final_list = pods.list(&ListParams::default()).await?;
    println!("Remaining pods: {}", final_list.items.len());

    // Cluster-scoped resource operations
    println!("\n=== Cluster-Scoped Resources (Nodes) ===\n");

    let nodes: Api<Node> = Api::all(client);

    // Create nodes
    println!("Creating nodes...");
    for i in 1..=3 {
        let zone = format!("zone-{}", (i % 2) + 1);
        let node = create_node(&format!("worker-{}", i), &zone);
        nodes.create(&PostParams::default(), &node).await?;
    }

    let node_list = nodes.list(&ListParams::default()).await?;
    println!("Created nodes: {}", node_list.items.len());
    for node in &node_list.items {
        println!(
            "  - {} (namespace: {:?})",
            node.metadata.name.as_ref().unwrap(),
            node.metadata.namespace
        );
    }

    // Filter nodes by label
    let zone1_nodes = nodes
        .list(&ListParams::default().labels("zone=zone-1"))
        .await?;
    println!("\nNodes in zone-1: {}", zone1_nodes.items.len());

    // Patch a node
    let node_patch = json!({
        "metadata": {
            "labels": {
                "maintenance": "true"
            }
        }
    });
    let patched_node = nodes
        .patch(
            "worker-1",
            &PatchParams::default(),
            &Patch::Merge(&node_patch),
        )
        .await?;
    println!("\nPatched worker-1 with maintenance label");
    println!("Labels: {:?}", patched_node.metadata.labels);

    // Delete a node
    nodes.delete("worker-3", &DeleteParams::default()).await?;
    println!("\nDeleted worker-3");

    let remaining_nodes = nodes.list(&ListParams::default()).await?;
    println!("Remaining nodes: {}", remaining_nodes.items.len());

    Ok(())
}
