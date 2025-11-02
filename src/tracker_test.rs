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
    fn test_add_sets_resource_version_999() {
        let tracker = ObjectTracker::new();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "default");

        let added = tracker.add(&gvr, &gvk, obj, "default").unwrap();
        assert_eq!(added["metadata"]["name"], "test-pod");
        assert_eq!(added["metadata"]["resourceVersion"], "999");

        let retrieved = tracker.get(&gvr, "default", "test-pod").unwrap();
        assert_eq!(retrieved["metadata"]["resourceVersion"], "999");
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
        assert!(result.is_ok(), "Cluster-wide list should succeed with empty result");
        assert_eq!(result.unwrap().len(), 0, "List should return empty vector");
    }
}
