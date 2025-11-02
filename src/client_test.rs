#[cfg(test)]
mod tests {
    use crate::client::*;
    use k8s_openapi::api::core::v1::Pod;
    use kube::api::{ListParams, PatchParams, PostParams};

    #[test]
    fn test_create_and_get() {
        let client = FakeClient::new();
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());

        let created = client
            .create("default", &pod, &PostParams::default())
            .unwrap();

        assert_eq!(created.metadata.name, Some("test-pod".to_string()));
        assert_eq!(created.metadata.resource_version, Some("1".to_string()));

        let retrieved: Pod = client.get("default", "test-pod").unwrap();
        assert_eq!(retrieved.metadata.name, Some("test-pod".to_string()));
    }

    #[test]
    fn test_list() {
        let client = FakeClient::new();

        for i in 1..=3 {
            let mut pod = Pod::default();
            pod.metadata.name = Some(format!("pod-{}", i));
            pod.metadata.namespace = Some("default".to_string());
            client
                .create("default", &pod, &PostParams::default())
                .unwrap();
        }

        let pods: Vec<Pod> = client
            .list(Some("default"), &ListParams::default())
            .unwrap();

        assert_eq!(pods.len(), 3);
    }

    #[test]
    fn test_readme_patch_example() {
        use serde_json::json;

        let client = FakeClient::new();

        // Create a pod first
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());

        client
            .create("default", &pod, &PostParams::default())
            .unwrap();

        // Test the patch example from README
        let patch = json!({
            "metadata": {
                "labels": {
                    "new-label": "value"
                }
            }
        });

        let patched: Pod = client
            .patch("default", "test-pod", &patch, &PatchParams::default())
            .unwrap();

        assert_eq!(
            patched
                .metadata
                .labels
                .as_ref()
                .unwrap()
                .get("new-label")
                .unwrap(),
            "value"
        );
    }

    #[test]
    fn test_resource_version_conflict_detection() {
        let client = FakeClient::new();

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());

        let created = client
            .create("default", &pod, &PostParams::default())
            .unwrap();
        assert_eq!(created.metadata.resource_version, Some("1".to_string()));

        // Get two copies of the same pod
        let mut pod1: Pod = client.get("default", "test-pod").unwrap();
        let mut pod2: Pod = client.get("default", "test-pod").unwrap();

        // Update pod1 successfully
        pod1.metadata.labels = Some(
            [("test".to_string(), "value1".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        let updated1 = client
            .update("default", &pod1, &PostParams::default())
            .unwrap();
        assert_eq!(updated1.metadata.resource_version, Some("2".to_string()));

        // Try to update pod2 (still has RV=1) - should fail with conflict
        pod2.metadata.labels = Some(
            [("test".to_string(), "value2".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        let result = client.update("default", &pod2, &PostParams::default());

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::Error::Conflict(_)));
    }

    #[test]
    fn test_resource_version_increments() {
        let client = FakeClient::new();

        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());

        // Create: RV should be 1
        let created = client
            .create("default", &pod, &PostParams::default())
            .unwrap();
        assert_eq!(created.metadata.resource_version, Some("1".to_string()));

        // Update 1: RV should be 2
        let mut updated = created.clone();
        updated.metadata.labels = Some(
            [("v".to_string(), "1".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        let updated = client
            .update("default", &updated, &PostParams::default())
            .unwrap();
        assert_eq!(updated.metadata.resource_version, Some("2".to_string()));

        // Update 2: RV should be 3
        let mut updated = updated.clone();
        updated.metadata.labels = Some(
            [("v".to_string(), "2".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        let updated = client
            .update("default", &updated, &PostParams::default())
            .unwrap();
        assert_eq!(updated.metadata.resource_version, Some("3".to_string()));
    }
}
