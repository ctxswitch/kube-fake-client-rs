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

    #[test]
    fn test_field_selector_metadata_name() {
        let client = FakeClient::new();

        // Create multiple pods
        for i in 1..=3 {
            let mut pod = Pod::default();
            pod.metadata.name = Some(format!("pod-{}", i));
            pod.metadata.namespace = Some("default".to_string());
            client
                .create("default", &pod, &PostParams::default())
                .unwrap();
        }

        // Filter by metadata.name without registering an index
        let params = ListParams::default().fields("metadata.name=pod-2");
        let pods: Vec<Pod> = client.list(Some("default"), &params).unwrap();

        assert_eq!(pods.len(), 1);
        assert_eq!(pods[0].metadata.name, Some("pod-2".to_string()));
    }

    #[test]
    fn test_field_selector_metadata_namespace() {
        let client = FakeClient::new();

        // Create pods in different namespaces
        let mut pod1 = Pod::default();
        pod1.metadata.name = Some("pod-1".to_string());
        pod1.metadata.namespace = Some("default".to_string());
        client
            .create("default", &pod1, &PostParams::default())
            .unwrap();

        let mut pod2 = Pod::default();
        pod2.metadata.name = Some("pod-2".to_string());
        pod2.metadata.namespace = Some("kube-system".to_string());
        client
            .create("kube-system", &pod2, &PostParams::default())
            .unwrap();

        // List all pods and filter by namespace using field selector
        let params = ListParams::default().fields("metadata.namespace=default");
        let pods: Vec<Pod> = client.list(None, &params).unwrap();

        assert_eq!(pods.len(), 1);
        assert_eq!(pods[0].metadata.namespace, Some("default".to_string()));
    }

    #[test]
    fn test_field_selector_spec_nodename_for_pods() {
        let client = FakeClient::new();

        // Create pods with different node names
        let mut pod1 = Pod::default();
        pod1.metadata.name = Some("pod-1".to_string());
        pod1.metadata.namespace = Some("default".to_string());
        pod1.spec = Some(Default::default());
        if let Some(ref mut spec) = pod1.spec {
            spec.node_name = Some("node-1".to_string());
        }
        client
            .create("default", &pod1, &PostParams::default())
            .unwrap();

        let mut pod2 = Pod::default();
        pod2.metadata.name = Some("pod-2".to_string());
        pod2.metadata.namespace = Some("default".to_string());
        pod2.spec = Some(Default::default());
        if let Some(ref mut spec) = pod2.spec {
            spec.node_name = Some("node-2".to_string());
        }
        client
            .create("default", &pod2, &PostParams::default())
            .unwrap();

        // Filter by spec.nodeName - this is pre-registered for Pods
        let params = ListParams::default().fields("spec.nodeName=node-1");
        let pods: Vec<Pod> = client.list(Some("default"), &params).unwrap();

        assert_eq!(pods.len(), 1);
        assert_eq!(pods[0].metadata.name, Some("pod-1".to_string()));
        assert_eq!(
            pods[0].spec.as_ref().unwrap().node_name,
            Some("node-1".to_string())
        );
    }

    #[test]
    fn test_field_selector_status_phase_for_pods() {
        let client = FakeClient::new();

        // Create pods with different phases
        let mut pod1 = Pod::default();
        pod1.metadata.name = Some("pod-1".to_string());
        pod1.metadata.namespace = Some("default".to_string());
        pod1.status = Some(Default::default());
        if let Some(ref mut status) = pod1.status {
            status.phase = Some("Running".to_string());
        }
        client
            .create("default", &pod1, &PostParams::default())
            .unwrap();

        let mut pod2 = Pod::default();
        pod2.metadata.name = Some("pod-2".to_string());
        pod2.metadata.namespace = Some("default".to_string());
        pod2.status = Some(Default::default());
        if let Some(ref mut status) = pod2.status {
            status.phase = Some("Pending".to_string());
        }
        client
            .create("default", &pod2, &PostParams::default())
            .unwrap();

        // Filter by status.phase - this is pre-registered for Pods
        let params = ListParams::default().fields("status.phase=Running");
        let pods: Vec<Pod> = client.list(Some("default"), &params).unwrap();

        assert_eq!(pods.len(), 1);
        assert_eq!(pods[0].metadata.name, Some("pod-1".to_string()));
    }

    #[test]
    fn test_field_selector_multiple_common_fields() {
        let client = FakeClient::new();

        // Create multiple pods
        for i in 1..=3 {
            let mut pod = Pod::default();
            pod.metadata.name = Some(format!("pod-{}", i));
            pod.metadata.namespace = Some("default".to_string());
            client
                .create("default", &pod, &PostParams::default())
                .unwrap();
        }

        // Create a pod in a different namespace
        let mut pod = Pod::default();
        pod.metadata.name = Some("pod-1".to_string());
        pod.metadata.namespace = Some("kube-system".to_string());
        client
            .create("kube-system", &pod, &PostParams::default())
            .unwrap();

        // Filter by both metadata.name and metadata.namespace
        let params = ListParams::default().fields("metadata.name=pod-1,metadata.namespace=default");
        let pods: Vec<Pod> = client.list(None, &params).unwrap();

        assert_eq!(pods.len(), 1);
        assert_eq!(pods[0].metadata.name, Some("pod-1".to_string()));
        assert_eq!(pods[0].metadata.namespace, Some("default".to_string()));
    }

    #[test]
    fn test_field_selector_no_match() {
        let client = FakeClient::new();

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());
        client
            .create("default", &pod, &PostParams::default())
            .unwrap();

        // Filter by non-existent name
        let params = ListParams::default().fields("metadata.name=nonexistent");
        let pods: Vec<Pod> = client.list(Some("default"), &params).unwrap();

        assert_eq!(pods.len(), 0);
    }
}
