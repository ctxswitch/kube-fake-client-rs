#[cfg(test)]
mod tests {
    use crate::tracker::*;
    use serde_json::json;

    fn create_test_object(name: &str, namespace: &str) -> serde_json::Value {
        json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": name,
                "namespace": namespace,
            },
            "spec": {
                "containers": [{
                    "name": "test",
                    "image": "nginx"
                }]
            }
        })
    }

    #[test]
    fn test_add_sets_globally_increasing_resource_version() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "default");

        let added = tracker.add(&gvr, &gvk, obj, "default").unwrap();
        assert_eq!(added["metadata"]["name"], "test-pod");
        // Should have a resource version set (globally increasing)
        let rv1 = added["metadata"]["resourceVersion"].as_str().unwrap();
        assert!(!rv1.is_empty());

        let retrieved = tracker.get(&gvr, "default", "test-pod").unwrap();
        assert_eq!(retrieved["metadata"]["resourceVersion"], rv1);

        // Add another object and verify RV increases
        let obj2 = create_test_object("test-pod-2", "default");
        let added2 = tracker.add(&gvr, &gvk, obj2, "default").unwrap();
        let rv2 = added2["metadata"]["resourceVersion"].as_str().unwrap();

        // Parse and compare to verify RV is globally increasing
        let rv1_num: u64 = rv1.parse().unwrap();
        let rv2_num: u64 = rv2.parse().unwrap();
        assert!(
            rv2_num > rv1_num,
            "Resource version should be globally increasing"
        );
    }

    #[test]
    fn test_add_preserves_existing_resource_version() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let mut obj = create_test_object("test-pod", "default");
        obj["metadata"]["resourceVersion"] = json!("42");

        let added = tracker.add(&gvr, &gvk, obj, "default").unwrap();
        assert_eq!(added["metadata"]["resourceVersion"], "42");
    }

    #[test]
    fn test_add_replaces_existing_object() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");

        let obj1 = create_test_object("test-pod", "default");
        tracker.add(&gvr, &gvk, obj1, "default").unwrap();

        let mut obj2 = create_test_object("test-pod", "default");
        obj2["spec"]["containers"][0]["image"] = json!("nginx:latest");
        let added = tracker.add(&gvr, &gvk, obj2, "default").unwrap();

        assert_eq!(added["spec"]["containers"][0]["image"], "nginx:latest");
    }

    #[test]
    fn test_create_sets_resource_version_1() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "default");

        let created = tracker.create(&gvr, &gvk, obj, "default").unwrap();
        assert_eq!(created["metadata"]["name"], "test-pod");
        assert_eq!(created["metadata"]["resourceVersion"], "1");

        let retrieved = tracker.get(&gvr, "default", "test-pod").unwrap();
        assert_eq!(retrieved["metadata"]["name"], "test-pod");
    }

    #[test]
    fn test_create_errors_if_resource_version_set() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let mut obj = create_test_object("test-pod", "default");
        obj["metadata"]["resourceVersion"] = json!("1");

        let result = tracker.create(&gvr, &gvk, obj, "default");
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::Error::InvalidRequest(_))));

        if let Err(crate::Error::InvalidRequest(msg)) = result {
            assert!(msg.contains("resourceVersion can not be set"));
        }
    }

    #[test]
    fn test_add_errors_if_deletion_timestamp_without_finalizers() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let mut obj = create_test_object("test-pod", "default");
        obj["metadata"]["deletionTimestamp"] = json!("2024-01-01T00:00:00Z");

        let result = tracker.add(&gvr, &gvk, obj, "default");
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::Error::InvalidRequest(_))));

        if let Err(crate::Error::InvalidRequest(msg)) = result {
            assert!(msg.contains("deletionTimestamp but no finalizers"));
        }
    }

    #[test]
    fn test_add_allows_deletion_timestamp_with_finalizers() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let mut obj = create_test_object("test-pod", "default");
        obj["metadata"]["deletionTimestamp"] = json!("2024-01-01T00:00:00Z");
        obj["metadata"]["finalizers"] = json!(["test-finalizer"]);

        let result = tracker.add(&gvr, &gvk, obj, "default");
        assert!(result.is_ok());
    }

    #[test]
    fn test_update() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "default");

        tracker.create(&gvr, &gvk, obj, "default").unwrap();

        let mut updated_obj = create_test_object("test-pod", "default");
        updated_obj["metadata"]["resourceVersion"] = json!("1");
        updated_obj["spec"]["containers"][0]["image"] = json!("nginx:latest");

        let updated = tracker
            .update(&gvr, &gvk, updated_obj, "default", false)
            .unwrap();
        assert_eq!(updated["metadata"]["resourceVersion"], "2");
        assert_eq!(updated["spec"]["containers"][0]["image"], "nginx:latest");
    }

    #[test]
    fn test_delete() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "default");

        tracker.create(&gvr, &gvk, obj, "default").unwrap();
        tracker.delete(&gvr, "default", "test-pod").unwrap();

        assert!(tracker.get(&gvr, "default", "test-pod").is_err());
    }

    #[test]
    fn test_list() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");

        tracker
            .create(&gvr, &gvk, create_test_object("pod1", "default"), "default")
            .unwrap();
        tracker
            .create(&gvr, &gvk, create_test_object("pod2", "default"), "default")
            .unwrap();
        tracker
            .create(&gvr, &gvk, create_test_object("pod3", "other"), "other")
            .unwrap();

        let default_list = tracker.list(&gvr, Some("default")).unwrap();
        assert_eq!(default_list.len(), 2);

        let all_list = tracker.list(&gvr, None).unwrap();
        assert_eq!(all_list.len(), 3);
    }

    #[test]
    fn test_list_empty_returns_empty_list() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");

        // List when no objects of this type exist should return empty list, not error
        let result = tracker.list(&gvr, Some("default"));
        assert!(result.is_ok(), "List should succeed with empty result");
        assert_eq!(result.unwrap().len(), 0, "List should return empty vector");

        // Same for cluster-wide list
        let result = tracker.list(&gvr, None);
        assert!(
            result.is_ok(),
            "Cluster-wide list should succeed with empty result"
        );
        assert_eq!(result.unwrap().len(), 0, "List should return empty vector");
    }

    #[test]
    fn test_generation_initialized_on_create() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "default");

        let created = tracker.create(&gvr, &gvk, obj, "default").unwrap();
        assert_eq!(created["metadata"]["generation"], 1);
    }

    #[test]
    fn test_generation_increments_on_spec_update() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "default");

        let created = tracker.create(&gvr, &gvk, obj, "default").unwrap();
        assert_eq!(created["metadata"]["generation"], 1);

        let mut updated_obj = create_test_object("test-pod", "default");
        updated_obj["metadata"]["resourceVersion"] = json!("1");
        updated_obj["spec"]["containers"][0]["image"] = json!("nginx:latest");

        let updated = tracker
            .update(&gvr, &gvk, updated_obj, "default", false)
            .unwrap();
        assert_eq!(updated["metadata"]["generation"], 2);
    }

    #[test]
    fn test_generation_not_incremented_on_status_update() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        tracker.add_status_subresource(gvk.clone());

        let obj = create_test_object("test-pod", "default");
        let created = tracker.create(&gvr, &gvk, obj, "default").unwrap();
        assert_eq!(created["metadata"]["generation"], 1);

        let mut status_update = create_test_object("test-pod", "default");
        status_update["metadata"]["resourceVersion"] = json!("1");
        status_update["status"] = json!({"phase": "Running"});

        let updated = tracker
            .update(&gvr, &gvk, status_update, "default", true)
            .unwrap();
        assert_eq!(updated["metadata"]["generation"], 1);
    }

    #[test]
    fn test_generation_multiple_increments() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "default");

        let created = tracker.create(&gvr, &gvk, obj, "default").unwrap();
        assert_eq!(created["metadata"]["generation"], 1);

        // First spec update
        let mut updated_obj = create_test_object("test-pod", "default");
        updated_obj["metadata"]["resourceVersion"] = json!("1");
        updated_obj["spec"]["containers"][0]["image"] = json!("nginx:1.19");
        let updated = tracker
            .update(&gvr, &gvk, updated_obj, "default", false)
            .unwrap();
        assert_eq!(updated["metadata"]["generation"], 2);

        // Second spec update
        let mut updated_obj = create_test_object("test-pod", "default");
        updated_obj["metadata"]["resourceVersion"] = json!("2");
        updated_obj["spec"]["containers"][0]["image"] = json!("nginx:1.20");
        let updated = tracker
            .update(&gvr, &gvk, updated_obj, "default", false)
            .unwrap();
        assert_eq!(updated["metadata"]["generation"], 3);
    }

    #[test]
    fn test_resource_version_globally_increasing_across_types() {
        let tracker = ObjectTracker::new();

        // Create a Pod
        let pod_gvr = GVR::new("", "v1", "pods");
        let pod_gvk = GVK::new("", "v1", "Pod");
        let pod = create_test_object("test-pod", "default");
        let created_pod = tracker.create(&pod_gvr, &pod_gvk, pod, "default").unwrap();
        let rv1: u64 = created_pod["metadata"]["resourceVersion"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        // Create a ConfigMap
        let cm_gvr = GVR::new("", "v1", "configmaps");
        let cm_gvk = GVK::new("", "v1", "ConfigMap");
        let cm = json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {
                "name": "test-cm",
                "namespace": "default",
            },
            "data": {
                "key": "value"
            }
        });
        let created_cm = tracker.create(&cm_gvr, &cm_gvk, cm, "default").unwrap();
        let rv2: u64 = created_cm["metadata"]["resourceVersion"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        // Create a Service
        let svc_gvr = GVR::new("", "v1", "services");
        let svc_gvk = GVK::new("", "v1", "Service");
        let svc = json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": {
                "name": "test-svc",
                "namespace": "default",
            },
            "spec": {
                "ports": [{
                    "port": 80
                }]
            }
        });
        let created_svc = tracker.create(&svc_gvr, &svc_gvk, svc, "default").unwrap();
        let rv3: u64 = created_svc["metadata"]["resourceVersion"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        // Verify globally increasing across all resource types
        assert!(
            rv2 > rv1,
            "ConfigMap RV ({}) should be > Pod RV ({})",
            rv2,
            rv1
        );
        assert!(
            rv3 > rv2,
            "Service RV ({}) should be > ConfigMap RV ({})",
            rv3,
            rv2
        );
        assert!(
            rv3 > rv1,
            "Service RV ({}) should be > Pod RV ({})",
            rv3,
            rv1
        );
    }

    #[test]
    fn test_auto_register_status_subresource_on_create() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");

        // Initially no status subresource registered
        assert!(!tracker.has_status_subresource(&gvk));

        // Create a Pod with a status field
        let mut obj = create_test_object("test-pod", "default");
        obj["status"] = json!({"phase": "Pending"});

        tracker.create(&gvr, &gvk, obj, "default").unwrap();

        // Status subresource should be automatically registered
        assert!(tracker.has_status_subresource(&gvk));
    }

    #[test]
    fn test_auto_register_status_subresource_on_add() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");

        // Initially no status subresource registered
        assert!(!tracker.has_status_subresource(&gvk));

        // Add a Pod with a status field
        let mut obj = create_test_object("test-pod", "default");
        obj["status"] = json!({"phase": "Running"});

        tracker.add(&gvr, &gvk, obj, "default").unwrap();

        // Status subresource should be automatically registered
        assert!(tracker.has_status_subresource(&gvk));
    }

    #[test]
    fn test_no_auto_register_without_status_field() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "configmaps");
        let gvk = GVK::new("", "v1", "ConfigMap");

        // Initially no status subresource registered
        assert!(!tracker.has_status_subresource(&gvk));

        // Create a ConfigMap without a status field
        let obj = json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {
                "name": "test-cm",
                "namespace": "default",
            },
            "data": {
                "key": "value"
            }
        });

        tracker.create(&gvr, &gvk, obj, "default").unwrap();

        // Status subresource should NOT be registered
        assert!(!tracker.has_status_subresource(&gvk));
    }

    #[test]
    fn test_status_subresource_prevents_status_modification_on_regular_update() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");

        // Create a Pod with status - this auto-registers status subresource
        let mut obj = create_test_object("test-pod", "default");
        obj["status"] = json!({"phase": "Pending"});

        let created = tracker.create(&gvr, &gvk, obj, "default").unwrap();
        assert_eq!(created["status"]["phase"], "Pending");

        // Verify status subresource was auto-registered
        assert!(tracker.has_status_subresource(&gvk));

        // Try to update spec AND status in a regular update
        let mut updated_obj = create_test_object("test-pod", "default");
        updated_obj["metadata"]["resourceVersion"] = json!("1");
        updated_obj["spec"]["containers"][0]["image"] = json!("nginx:latest");
        updated_obj["status"] = json!({"phase": "Running"}); // Try to change status

        let updated = tracker
            .update(&gvr, &gvk, updated_obj, "default", false)
            .unwrap();

        // Spec should be updated
        assert_eq!(updated["spec"]["containers"][0]["image"], "nginx:latest");
        // Status should NOT be updated (preserved from original)
        assert_eq!(updated["status"]["phase"], "Pending");
    }
}
