//! Loading fixtures from YAML files

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use kube::api::{Api, ListParams};
use kube_fake_client::ClientBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Fixtures can contain multiple resources across different namespaces in a single file
    let client = ClientBuilder::new()
        .with_fixture_dir("fixtures")
        .load_fixtures_or_panic(["pods.yaml", "deployment.yaml", "configmap.yaml"])
        .build()
        .await?;

    println!("Pods in default namespace:");
    let pods: Api<Pod> = Api::namespaced(client.clone(), "default");
    let pod_list = pods.list(&ListParams::default()).await?;
    for pod in &pod_list.items {
        println!("  - {}", pod.metadata.name.as_ref().unwrap());
    }

    println!("\nPods in cache namespace:");
    let cache_pods: Api<Pod> = Api::namespaced(client.clone(), "cache");
    let cache_pod_list = cache_pods.list(&ListParams::default()).await?;
    for pod in &cache_pod_list.items {
        println!("  - {}", pod.metadata.name.as_ref().unwrap());
    }

    println!("\nDeployments in production namespace:");
    let deployments: Api<Deployment> = Api::namespaced(client.clone(), "production");
    let deployment_list = deployments.list(&ListParams::default()).await?;
    for deployment in &deployment_list.items {
        let replicas = deployment
            .spec
            .as_ref()
            .and_then(|s| s.replicas)
            .unwrap_or(0);
        println!(
            "  - {} (replicas: {})",
            deployment.metadata.name.as_ref().unwrap(),
            replicas
        );
    }

    println!("\nRetrieving specific resources:");
    let nginx_pod = pods.get("nginx-pod").await?;
    println!("  Pod: {}", nginx_pod.metadata.name.as_ref().unwrap());

    let prod_deployments: Api<Deployment> = Api::namespaced(client.clone(), "production");
    let deployment = prod_deployments.get("web-deployment").await?;
    println!(
        "  Deployment: {}",
        deployment.metadata.name.as_ref().unwrap()
    );

    let configmaps: Api<ConfigMap> = Api::namespaced(client, "default");
    let config = configmaps.get("app-config").await?;
    println!("  ConfigMap: {}", config.metadata.name.as_ref().unwrap());

    Ok(())
}
