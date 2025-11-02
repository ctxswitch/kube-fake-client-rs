//! Deployment status controller using status subresource

use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec, DeploymentStatus};
use k8s_openapi::api::core::v1::PodTemplateSpec;
use kube::api::{Api, PostParams};
use kube_fake_client::ClientBuilder;

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
    let deployment = Deployment {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some("nginx-deployment".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            replicas: Some(3),
            selector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector {
                match_labels: Some(
                    [("app".to_string(), "nginx".to_string())]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                ..Default::default()
            },
            template: PodTemplateSpec {
                metadata: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                    labels: Some(
                        [("app".to_string(), "nginx".to_string())]
                            .iter()
                            .cloned()
                            .collect(),
                    ),
                    ..Default::default()
                }),
                spec: None,
            },
            ..Default::default()
        }),
        status: None,
    };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_status_update_does_not_affect_spec() {
        let deployment = Deployment {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some("test-deployment".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(5),
                selector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector::default(),
                template: PodTemplateSpec::default(),
                ..Default::default()
            }),
            status: None,
        };

        let client = ClientBuilder::new()
            .with_status_subresource::<Deployment>()
            .with_object(deployment)
            .build()
            .await
            .unwrap();

        let api: Api<Deployment> = Api::namespaced(client, "default");
        let controller = DeploymentStatusController::new(api.clone());

        controller.reconcile("test-deployment").await.unwrap();

        let updated = api.get("test-deployment").await.unwrap();
        assert_eq!(
            updated.spec.as_ref().unwrap().replicas,
            Some(5),
            "Spec replicas should not be modified by status update"
        );
        assert_eq!(
            updated.status.as_ref().unwrap().replicas,
            Some(5),
            "Status should be updated"
        );
    }

    #[tokio::test]
    async fn test_spec_update_does_not_affect_status() {
        let mut deployment = Deployment {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some("test-deployment".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(3),
                selector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector::default(),
                template: PodTemplateSpec::default(),
                ..Default::default()
            }),
            status: Some(DeploymentStatus {
                replicas: Some(3),
                ready_replicas: Some(3),
                available_replicas: Some(3),
                ..Default::default()
            }),
        };

        let client = ClientBuilder::new()
            .with_status_subresource::<Deployment>()
            .with_object(deployment.clone())
            .build()
            .await
            .unwrap();

        let api: Api<Deployment> = Api::namespaced(client, "default");

        deployment.spec.as_mut().unwrap().replicas = Some(5);
        api.replace("test-deployment", &PostParams::default(), &deployment)
            .await
            .unwrap();

        let updated = api.get("test-deployment").await.unwrap();
        assert_eq!(
            updated.spec.as_ref().unwrap().replicas,
            Some(5),
            "Spec should be updated"
        );
        assert_eq!(
            updated.status.as_ref().unwrap().replicas,
            Some(3),
            "Status should not be affected by spec update"
        );
    }

    #[tokio::test]
    async fn test_controller_handles_missing_replicas() {
        let deployment = Deployment {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some("test-deployment".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: None,
                selector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector::default(),
                template: PodTemplateSpec::default(),
                ..Default::default()
            }),
            status: None,
        };

        let client = ClientBuilder::new()
            .with_status_subresource::<Deployment>()
            .with_object(deployment)
            .build()
            .await
            .unwrap();

        let api: Api<Deployment> = Api::namespaced(client, "default");
        let controller = DeploymentStatusController::new(api.clone());

        controller.reconcile("test-deployment").await.unwrap();

        let updated = api.get("test-deployment").await.unwrap();
        let status = updated.status.as_ref().unwrap();
        assert_eq!(status.replicas, Some(1));
        assert_eq!(status.ready_replicas, Some(1));
    }

    #[tokio::test]
    async fn test_status_subresource_increments_resource_version() {
        let deployment = Deployment {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some("test-deployment".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(2),
                selector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector::default(),
                template: PodTemplateSpec::default(),
                ..Default::default()
            }),
            status: None,
        };

        let client = ClientBuilder::new()
            .with_status_subresource::<Deployment>()
            .with_object(deployment)
            .build()
            .await
            .unwrap();

        let api: Api<Deployment> = Api::namespaced(client, "default");
        let controller = DeploymentStatusController::new(api.clone());

        let initial = api.get("test-deployment").await.unwrap();
        let initial_rv = initial.metadata.resource_version.clone();

        controller.reconcile("test-deployment").await.unwrap();

        let updated = api.get("test-deployment").await.unwrap();
        let updated_rv = updated.metadata.resource_version.clone();

        assert_ne!(
            initial_rv, updated_rv,
            "Resource version should change after status update"
        );
    }

    #[tokio::test]
    async fn test_multiple_status_updates() {
        let deployment = Deployment {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some("test-deployment".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(3),
                selector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector::default(),
                template: PodTemplateSpec::default(),
                ..Default::default()
            }),
            status: None,
        };

        let client = ClientBuilder::new()
            .with_status_subresource::<Deployment>()
            .with_object(deployment)
            .build()
            .await
            .unwrap();

        let api: Api<Deployment> = Api::namespaced(client, "default");
        let controller = DeploymentStatusController::new(api.clone());

        controller.reconcile("test-deployment").await.unwrap();
        controller.reconcile("test-deployment").await.unwrap();
        controller.reconcile("test-deployment").await.unwrap();

        let final_deployment = api.get("test-deployment").await.unwrap();
        assert!(final_deployment.status.is_some());

        let rv: u32 = final_deployment
            .metadata
            .resource_version
            .unwrap()
            .parse()
            .unwrap();
        assert!(rv >= 3, "Resource version should reflect multiple updates");
    }
}
