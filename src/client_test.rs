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
        assert!(matches!(err, crate::Error::Conflict(_)), "Expected Conflict error, got: {:?}", err);
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

    #[test]
    fn test_verb_validation_unsupported_verb() {
        use k8s_openapi::api::core::v1::ComponentStatus;

        let client = FakeClient::new();

        // ComponentStatus only supports get/list, not create
        // Based on discovery data, ComponentStatus doesn't support create verb
        let mut cs = ComponentStatus::default();
        cs.metadata.name = Some("test-cs".to_string());

        // Try to create a ComponentStatus - should fail with VerbNotSupported
        let result = client.create("", &cs, &PostParams::default());

        match result {
            Err(crate::Error::VerbNotSupported { verb, kind }) => {
                assert_eq!(verb, "create");
                assert_eq!(kind, "ComponentStatus");
            }
            Ok(_) => panic!("Expected VerbNotSupported error, got success"),
            Err(e) => panic!("Expected VerbNotSupported error, got: {:?}", e),
        }
    }

    #[test]
    fn test_verb_validation_supported_verbs() {
        let client = FakeClient::new();

        // Create a pod (create verb supported)
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());

        let result = client.create("default", &pod, &PostParams::default());
        assert!(result.is_ok(), "Create should succeed for Pod");

        // Get the pod (get verb supported)
        let result = client.get::<Pod>("default", "test-pod");
        assert!(result.is_ok(), "Get should succeed for Pod");

        // List pods (list verb supported)
        let result = client.list::<Pod>(Some("default"), &ListParams::default());
        assert!(result.is_ok(), "List should succeed for Pod");

        // Update the pod (update verb supported)
        let mut updated_pod = pod.clone();
        updated_pod.metadata.labels = Some(
            [("test".to_string(), "value".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        let result = client.update("default", &updated_pod, &PostParams::default());
        assert!(result.is_ok(), "Update should succeed for Pod");

        // Patch the pod (patch verb supported)
        let patch = serde_json::json!({"metadata": {"labels": {"patched": "true"}}});
        let result = client.patch::<Pod>("default", "test-pod", &patch, &PatchParams::default());
        assert!(result.is_ok(), "Patch should succeed for Pod");

        // Delete the pod (delete verb supported)
        let result = client.delete::<Pod>("default", "test-pod");
        assert!(result.is_ok(), "Delete should succeed for Pod");
    }

    #[test]
    fn test_verb_validation_for_crds() {
        // Verb validation for CRDs is tested through the builder_test::test_crd_registration
        // test which creates, gets, lists, and manipulates CRD instances.
        // All standard verbs (create, get, list, update, patch, delete) are allowed
        // for registered CRDs by default.
    }

    #[test]
    fn test_immutable_field_validation_metadata_name() {
        let client = FakeClient::new();

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());

        let created = client
            .create("default", &pod, &PostParams::default())
            .unwrap();

        // Try to update the pod with a different name (immutable field)
        let mut updated_pod = created.clone();
        updated_pod.metadata.name = Some("different-name".to_string());

        let result = client.update("default", &updated_pod, &PostParams::default());

        // Should fail with ImmutableField error
        match result {
            Err(crate::Error::ImmutableField { field }) => {
                assert_eq!(field, "metadata.name");
            }
            Ok(_) => panic!("Expected ImmutableField error, got success"),
            Err(e) => panic!("Expected ImmutableField error, got: {:?}", e),
        }
    }

    #[test]
    fn test_immutable_field_validation_metadata_namespace() {
        let client = FakeClient::new();

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());

        let created = client
            .create("default", &pod, &PostParams::default())
            .unwrap();

        // Try to update the pod with a different namespace (immutable field)
        let mut updated_pod = created.clone();
        updated_pod.metadata.namespace = Some("kube-system".to_string());

        let result = client.update("default", &updated_pod, &PostParams::default());

        // Should fail with ImmutableField error
        match result {
            Err(crate::Error::ImmutableField { field }) => {
                assert_eq!(field, "metadata.namespace");
            }
            Ok(_) => panic!("Expected ImmutableField error, got success"),
            Err(e) => panic!("Expected ImmutableField error, got: {:?}", e),
        }
    }

    #[test]
    fn test_immutable_field_validation_kind() {
        use k8s_openapi::api::core::v1::ConfigMap;

        let client = FakeClient::new();

        // Create a ConfigMap
        let mut cm = ConfigMap::default();
        cm.metadata.name = Some("test-cm".to_string());
        cm.metadata.namespace = Some("default".to_string());

        let created = client
            .create("default", &cm, &PostParams::default())
            .unwrap();

        // Manually manipulate the kind field to simulate changing it
        // (we can't do this through normal Rust types because of type safety)
        let mut cm_value = serde_json::to_value(&created).unwrap();
        cm_value["kind"] = serde_json::json!("DifferentKind");

        // Try to update via the internal tracker directly to bypass type checking
        // This simulates what would happen if someone tried to change the kind
        let gvr = crate::tracker::GVR::new("", "v1", "configmaps");
        let gvk = crate::tracker::GVK::new("", "v1", "ConfigMap");

        // Get existing to compare
        let existing = client.tracker.get(&gvr, "default", "test-cm").unwrap();

        // Validate immutable fields - should detect kind change
        let result = client.validate_immutable_fields(&gvk, &existing, &cm_value);

        // Should fail with ImmutableField error
        match result {
            Err(crate::Error::ImmutableField { field }) => {
                assert_eq!(field, "kind");
            }
            Ok(_) => panic!("Expected ImmutableField error, got success"),
            Err(e) => panic!("Expected ImmutableField error, got: {:?}", e),
        }
    }

    #[test]
    fn test_immutable_field_validation_allows_mutable_fields() {
        let client = FakeClient::new();

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());

        let created = client
            .create("default", &pod, &PostParams::default())
            .unwrap();

        // Update mutable fields (labels) - should succeed
        let mut updated_pod = created.clone();
        updated_pod.metadata.labels = Some(
            [("test".to_string(), "value".to_string())]
                .iter()
                .cloned()
                .collect(),
        );

        let result = client.update("default", &updated_pod, &PostParams::default());
        assert!(result.is_ok(), "Updating mutable fields should succeed");
    }

    #[test]
    fn test_immutable_field_validation_with_patch() {
        let client = FakeClient::new();

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());

        client
            .create("default", &pod, &PostParams::default())
            .unwrap();

        // Try to patch the name (immutable field)
        let patch = serde_json::json!({
            "metadata": {
                "name": "different-name"
            }
        });

        let result = client.patch::<Pod>("default", "test-pod", &patch, &PatchParams::default());

        // Should fail with ImmutableField error
        match result {
            Err(crate::Error::ImmutableField { field }) => {
                assert_eq!(field, "metadata.name");
            }
            Ok(_) => panic!("Expected ImmutableField error, got success"),
            Err(e) => panic!("Expected ImmutableField error, got: {:?}", e),
        }
    }

    #[test]
    fn test_immutable_field_validation_patch_allows_mutable() {
        let client = FakeClient::new();

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.namespace = Some("default".to_string());

        client
            .create("default", &pod, &PostParams::default())
            .unwrap();

        // Patch mutable field (labels) - should succeed
        let patch = serde_json::json!({
            "metadata": {
                "labels": {
                    "patched": "true"
                }
            }
        });

        let result = client.patch::<Pod>("default", "test-pod", &patch, &PatchParams::default());
        assert!(result.is_ok(), "Patching mutable fields should succeed");
    }
}
