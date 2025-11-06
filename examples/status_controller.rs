//! Deployment status controller using status subresource
//!
//! Example demonstrating how to use status subresources to separate spec and status updates.
//! This pattern is essential for controllers that need to update resource status without
//! modifying the desired state (spec).

use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec, DeploymentStatus};
use k8s_openapi::api::core::v1::PodTemplateSpec;
use kube::api::{Api, PostParams};
use kube_fake_client::ClientBuilder;
use std::collections::BTreeMap;

/// Create a deployment with the given name, namespace, and replicas
fn create_deployment(name: &str, namespace: &str, replicas: i32, app_label: &str) -> Deployment {
    let labels: BTreeMap<String, String> = [(String::from("app"), String::from(app_label))]
        .iter()
        .cloned()
        .collect();

    Deployment {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            replicas: Some(replicas),
            selector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            template: PodTemplateSpec {
                metadata: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: None,
            },
            ..Default::default()
        }),
        status: None,
    }
}

pub struct DeploymentStatusController {
    api: Api<Deployment>,
}

impl DeploymentStatusController {
    pub fn new(api: Api<Deployment>) -> Self {
        Self { api }
    }

    pub async fn reconcile(&self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut deployment = self.api.get(name).await?;

        let desired_replicas = deployment
            .spec
            .as_ref()
            .and_then(|spec| spec.replicas)
            .unwrap_or(1);

        // Build the status reflecting the current desired state
        #[cfg(not(feature = "v1_33"))]
        let status = DeploymentStatus {
            replicas: Some(desired_replicas),
            ready_replicas: Some(desired_replicas),
            available_replicas: Some(desired_replicas),
            unavailable_replicas: Some(0),
            updated_replicas: Some(desired_replicas),
            observed_generation: deployment.metadata.generation,
            conditions: None,
            collision_count: None,
        };

        #[cfg(feature = "v1_33")]
        let status = DeploymentStatus {
            replicas: Some(desired_replicas),
            ready_replicas: Some(desired_replicas),
            available_replicas: Some(desired_replicas),
            unavailable_replicas: Some(0),
            updated_replicas: Some(desired_replicas),
            observed_generation: deployment.metadata.generation,
            conditions: None,
            collision_count: None,
            terminating_replicas: None,
        };

        deployment.status = Some(status);

        // replace_status updates only the status subresource, leaving spec unchanged
        self.api
            .replace_status(
                name,
                &PostParams::default(),
                serde_json::to_vec(&deployment)?,
            )
            .await?;

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let deployment = create_deployment("nginx-deployment", "default", 3, "nginx");

    // Status subresources must be enabled explicitly to separate spec and status updates
    let client = ClientBuilder::new()
        .with_status_subresource::<Deployment>()
        .with_object(deployment)
        .build()
        .await?;

    let api: Api<Deployment> = Api::namespaced(client.clone(), "default");
    let controller = DeploymentStatusController::new(api.clone());

    controller.reconcile("nginx-deployment").await?;

    let updated = api.get("nginx-deployment").await?;
    if let Some(status) = &updated.status {
        println!("Deployment status:");
        println!("  Replicas: {}", status.replicas.unwrap_or(0));
        println!("  Ready: {}", status.ready_replicas.unwrap_or(0));
    }

    if let Some(spec) = &updated.spec {
        println!("Deployment spec:");
        println!("  Desired replicas: {}", spec.replicas.unwrap_or(0));
    }

    Ok(())
}
