//! Basic kube::Api operations

use k8s_openapi::api::core::v1::{Container, Pod, PodSpec};
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

    let pods: Api<Pod> = Api::namespaced(client, "default");

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

    Ok(())
}
