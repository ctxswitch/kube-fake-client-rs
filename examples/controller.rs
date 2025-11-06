//! Pod controller testing

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, Patch, PatchParams};
use kube_fake_client::ClientBuilder;
use serde_json::json;
use std::collections::BTreeMap;

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
    let mut pod1 = Pod::default();
    pod1.metadata.name = Some("web-server".to_string());
    pod1.metadata.namespace = Some("default".to_string());

    let mut pod2 = Pod::default();
    pod2.metadata.name = Some("api-server".to_string());
    pod2.metadata.namespace = Some("default".to_string());
    let mut labels2 = BTreeMap::new();
    labels2.insert("app".to_string(), "backend".to_string());
    pod2.metadata.labels = Some(labels2);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_controller_adds_label() {
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());

        let client = ClientBuilder::new().with_object(pod).build().await.unwrap();
        let pods: Api<Pod> = Api::namespaced(client, "default");
        let controller = PodLabelController::new(pods.clone(), "test-controller".to_string());

        controller.reconcile("test-pod").await.unwrap();

        let pod = pods.get("test-pod").await.unwrap();
        let labels = pod.metadata.labels.as_ref().unwrap();
        assert_eq!(
            labels.get("managed-by"),
            Some(&"test-controller".to_string())
        );
    }

    #[tokio::test]
    async fn test_controller_preserves_labels() {
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());
        let mut labels = BTreeMap::new();
        labels.insert("app".to_string(), "myapp".to_string());
        labels.insert("env".to_string(), "prod".to_string());
        pod.metadata.labels = Some(labels);

        let client = ClientBuilder::new().with_object(pod).build().await.unwrap();
        let pods: Api<Pod> = Api::namespaced(client, "default");
        let controller = PodLabelController::new(pods.clone(), "test-controller".to_string());

        controller.reconcile("test-pod").await.unwrap();

        let pod = pods.get("test-pod").await.unwrap();
        let labels = pod.metadata.labels.as_ref().unwrap();
        assert_eq!(labels.get("app"), Some(&"myapp".to_string()));
        assert_eq!(labels.get("env"), Some(&"prod".to_string()));
        assert_eq!(
            labels.get("managed-by"),
            Some(&"test-controller".to_string())
        );
    }

    #[tokio::test]
    async fn test_reconcile_all() {
        let mut pod1 = Pod::default();
        pod1.metadata.name = Some("pod-1".to_string());
        pod1.metadata.namespace = Some("default".to_string());

        let mut pod2 = Pod::default();
        pod2.metadata.name = Some("pod-2".to_string());
        pod2.metadata.namespace = Some("default".to_string());

        let client = ClientBuilder::new()
            .with_objects(vec![pod1, pod2])
            .build()
            .await
            .unwrap();

        let pods: Api<Pod> = Api::namespaced(client, "default");
        let controller = PodLabelController::new(pods.clone(), "test-controller".to_string());

        controller.reconcile_all().await.unwrap();

        let pod_list = pods.list(&ListParams::default()).await.unwrap();
        assert_eq!(pod_list.items.len(), 2);

        for pod in pod_list.items {
            let labels = pod.metadata.labels.as_ref().unwrap();
            assert_eq!(
                labels.get("managed-by"),
                Some(&"test-controller".to_string())
            );
        }
    }
}
