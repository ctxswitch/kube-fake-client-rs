//! Pod controller testing
//!
//! Example demonstrating a simple Kubernetes controller that adds a "managed-by" label
//! to pods. This pattern is common in controllers that need to track which resources
//! they manage.

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, Patch, PatchParams};
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

pub struct PodLabelController {
    api: Api<Pod>,
    controller_name: String,
}

impl PodLabelController {
    pub fn new(api: Api<Pod>, controller_name: String) -> Self {
        Self {
            api,
            controller_name,
        }
    }

    pub async fn reconcile(&self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let pod = self.api.get(name).await?;

        let needs_label = pod
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get("managed-by"))
            .is_none();

        if needs_label {
            // Strategic merge patch preserves existing labels and adds new ones
            let patch = json!({
                "metadata": {
                    "labels": {
                        "managed-by": self.controller_name
                    }
                }
            });

            self.api
                .patch(name, &PatchParams::default(), &Patch::Merge(&patch))
                .await?;
        }

        Ok(())
    }

    pub async fn reconcile_all(&self) -> Result<(), Box<dyn std::error::Error>> {
        let pod_list = self.api.list(&ListParams::default()).await?;

        for pod in pod_list.items {
            if let Some(name) = &pod.metadata.name {
                self.reconcile(name).await?;
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pod1 = create_pod("web-server", "default", None);

    let mut labels2 = BTreeMap::new();
    labels2.insert("app".to_string(), "backend".to_string());
    let pod2 = create_pod("api-server", "default", Some(labels2));

    let client = ClientBuilder::new()
        .with_objects(vec![pod1, pod2])
        .build()
        .await?;

    let pods: Api<Pod> = Api::namespaced(client, "default");
    let controller = PodLabelController::new(pods.clone(), "example-controller".to_string());

    println!("Running controller...");
    controller.reconcile_all().await?;

    let pod_list = pods.list(&ListParams::default()).await?;
    println!("\nPods after reconciliation:");
    for pod in pod_list.items {
        let name = pod.metadata.name.as_ref().unwrap();
        let managed_by = pod
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("managed-by"));
        println!("  {}: managed-by = {:?}", name, managed_by);
    }

    Ok(())
}
