//! Basic kube::Api operations with both namespaced and cluster-scoped resources

use k8s_openapi::api::core::v1::{Container, Node, Pod, PodSpec};
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube_fake_client::ClientBuilder;
use serde_json::json;
use std::collections::BTreeMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut initial_pod = Pod::default();
    initial_pod.metadata.name = Some("web-server".to_string());
    initial_pod.metadata.namespace = Some("default".to_string());

    let mut api_pod = Pod::default();
    api_pod.metadata.name = Some("api-server".to_string());
    api_pod.metadata.namespace = Some("default".to_string());
    let mut labels = BTreeMap::new();
    labels.insert("app".to_string(), "backend".to_string());
    api_pod.metadata.labels = Some(labels);

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

    let mut new_pod = Pod::default();
    new_pod.metadata.name = Some("nginx".to_string());
    let mut labels = BTreeMap::new();
    labels.insert("app".to_string(), "frontend".to_string());
    labels.insert("env".to_string(), "dev".to_string());
    new_pod.metadata.labels = Some(labels);
    new_pod.spec = Some(PodSpec {
        containers: vec![Container {
            name: "nginx".to_string(),
            image: Some("nginx:latest".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    });

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
        let mut node = Node::default();
        node.metadata.name = Some(format!("worker-{}", i));
        let mut labels = BTreeMap::new();
        labels.insert("role".to_string(), "worker".to_string());
        labels.insert("zone".to_string(), format!("zone-{}", (i % 2) + 1));
        node.metadata.labels = Some(labels);
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
