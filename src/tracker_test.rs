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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let mut obj = create_test_object("test-pod", "default");
        obj["metadata"]["resourceVersion"] = json!("42");

        let added = tracker.add(&gvr, &gvk, obj, "default").unwrap();
        assert_eq!(added["metadata"]["resourceVersion"], "42");
    }

    #[test]
    fn test_add_replaces_existing_object() {
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "default");

        tracker.create(&gvr, &gvk, obj, "default").unwrap();
        tracker.delete(&gvr, "default", "test-pod", true).unwrap();

        assert!(tracker.get(&gvr, "default", "test-pod").is_err());
    }

    #[test]
    fn test_list() {
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "default");

        let created = tracker.create(&gvr, &gvk, obj, "default").unwrap();
        assert_eq!(created["metadata"]["generation"], 1);
    }

    #[test]
    fn test_generation_increments_on_spec_update() {
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();

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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
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
        let tracker = ObjectTracker::default();
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

    #[test]
    fn test_delete_cascades_to_dependents() {
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let ns = "default";

        // Create the owner pod
        let owner_obj = create_test_object("owner-pod", ns);
        let created_owner = tracker.create(&gvr, &gvk, owner_obj, ns).unwrap();
        let owner_uid = created_owner["metadata"]["uid"].as_str().unwrap();

        // Create two child pods with ownerReferences pointing to the owner
        for name in &["child-1", "child-2"] {
            let mut child = create_test_object(name, ns);
            child["metadata"]["ownerReferences"] = json!([{
                "apiVersion": "v1",
                "kind": "Pod",
                "name": "owner-pod",
                "uid": owner_uid
            }]);
            tracker.create(&gvr, &gvk, child, ns).unwrap();
        }

        // Delete the owner
        tracker.delete(&gvr, ns, "owner-pod", true).unwrap();

        // All three should be gone
        assert!(tracker.get(&gvr, ns, "owner-pod").is_err());
        assert!(tracker.get(&gvr, ns, "child-1").is_err());
        assert!(tracker.get(&gvr, ns, "child-2").is_err());
    }

    #[test]
    fn test_delete_cascades_recursively() {
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let ns = "default";

        // Create grandparent
        let grandparent = create_test_object("grandparent", ns);
        let created_gp = tracker.create(&gvr, &gvk, grandparent, ns).unwrap();
        let gp_uid = created_gp["metadata"]["uid"].as_str().unwrap();

        // Create parent with ownerRef -> grandparent
        let mut parent = create_test_object("parent", ns);
        parent["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "v1",
            "kind": "Pod",
            "name": "grandparent",
            "uid": gp_uid
        }]);
        let created_parent = tracker.create(&gvr, &gvk, parent, ns).unwrap();
        let parent_uid = created_parent["metadata"]["uid"].as_str().unwrap();

        // Create child with ownerRef -> parent
        let mut child = create_test_object("child", ns);
        child["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "v1",
            "kind": "Pod",
            "name": "parent",
            "uid": parent_uid
        }]);
        tracker.create(&gvr, &gvk, child, ns).unwrap();

        // Delete grandparent
        tracker.delete(&gvr, ns, "grandparent", true).unwrap();

        // All three should be gone
        assert!(tracker.get(&gvr, ns, "grandparent").is_err());
        assert!(tracker.get(&gvr, ns, "parent").is_err());
        assert!(tracker.get(&gvr, ns, "child").is_err());
    }

    #[test]
    fn test_delete_does_not_affect_unrelated() {
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let ns = "default";

        // Create owner
        let owner = create_test_object("owner", ns);
        let created_owner = tracker.create(&gvr, &gvk, owner, ns).unwrap();
        let owner_uid = created_owner["metadata"]["uid"].as_str().unwrap();

        // Create child with ownerRef -> owner
        let mut child = create_test_object("child", ns);
        child["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "v1",
            "kind": "Pod",
            "name": "owner",
            "uid": owner_uid
        }]);
        tracker.create(&gvr, &gvk, child, ns).unwrap();

        // Create unrelated pod (no ownerRef)
        let unrelated = create_test_object("unrelated", ns);
        tracker.create(&gvr, &gvk, unrelated, ns).unwrap();

        // Delete owner
        tracker.delete(&gvr, ns, "owner", true).unwrap();

        // Owner and child should be gone
        assert!(tracker.get(&gvr, ns, "owner").is_err());
        assert!(tracker.get(&gvr, ns, "child").is_err());

        // Unrelated should survive
        assert!(tracker.get(&gvr, ns, "unrelated").is_ok());
    }

    #[test]
    fn test_delete_cascades_across_resource_types() {
        let tracker = ObjectTracker::default();
        let ns = "default";

        // Create a Deployment as the owner
        let deploy_gvr = GVR::new("apps", "v1", "deployments");
        let deploy_gvk = GVK::new("apps", "v1", "Deployment");
        let deploy_obj = json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": { "name": "my-deploy", "namespace": ns },
            "spec": { "replicas": 1 }
        });
        let created_deploy = tracker
            .create(&deploy_gvr, &deploy_gvk, deploy_obj, ns)
            .unwrap();
        let deploy_uid = created_deploy["metadata"]["uid"].as_str().unwrap();

        // Create a Pod as the dependent with ownerRef -> deployment
        let pod_gvr = GVR::new("", "v1", "pods");
        let pod_gvk = GVK::new("", "v1", "Pod");
        let mut pod_obj = create_test_object("my-pod", ns);
        pod_obj["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "name": "my-deploy",
            "uid": deploy_uid
        }]);
        tracker.create(&pod_gvr, &pod_gvk, pod_obj, ns).unwrap();

        // Delete the deployment
        tracker.delete(&deploy_gvr, ns, "my-deploy", true).unwrap();

        // Both should be gone
        assert!(tracker.get(&deploy_gvr, ns, "my-deploy").is_err());
        assert!(tracker.get(&pod_gvr, ns, "my-pod").is_err());
    }

    #[test]
    fn test_add_rejects_namespaced_resource_without_namespace() {
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "");

        let result = tracker.add(&gvr, &gvk, obj, "");
        assert!(result.is_err());
        if let Err(crate::Error::InvalidRequest(msg)) = result {
            assert!(msg.contains("namespaced resource"), "got: {msg}");
        } else {
            panic!("expected InvalidRequest error");
        }
    }

    #[test]
    fn test_add_rejects_cluster_scoped_resource_with_namespace() {
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("", "v1", "namespaces");
        let gvk = GVK::new("", "v1", "Namespace");
        let obj = json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": {
                "name": "test-ns",
                "namespace": "default",
            }
        });

        let result = tracker.add(&gvr, &gvk, obj, "default");
        assert!(result.is_err());
        if let Err(crate::Error::InvalidRequest(msg)) = result {
            assert!(msg.contains("cluster-scoped resource"), "got: {msg}");
        } else {
            panic!("expected InvalidRequest error");
        }
    }

    #[test]
    fn test_create_rejects_namespaced_resource_without_namespace() {
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("", "v1", "pods");
        let gvk = GVK::new("", "v1", "Pod");
        let obj = create_test_object("test-pod", "");

        let result = tracker.create(&gvr, &gvk, obj, "");
        assert!(result.is_err());
        if let Err(crate::Error::InvalidRequest(msg)) = result {
            assert!(msg.contains("namespaced resource"), "got: {msg}");
        } else {
            panic!("expected InvalidRequest error");
        }
    }

    #[test]
    fn test_create_rejects_cluster_scoped_resource_with_namespace() {
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("", "v1", "namespaces");
        let gvk = GVK::new("", "v1", "Namespace");
        let obj = json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": {
                "name": "test-ns",
                "namespace": "default",
            }
        });

        let result = tracker.create(&gvr, &gvk, obj, "default");
        assert!(result.is_err());
        if let Err(crate::Error::InvalidRequest(msg)) = result {
            assert!(msg.contains("cluster-scoped resource"), "got: {msg}");
        } else {
            panic!("expected InvalidRequest error");
        }
    }

    #[test]
    fn test_add_allows_cluster_scoped_resource_without_namespace() {
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("", "v1", "namespaces");
        let gvk = GVK::new("", "v1", "Namespace");
        let obj = json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": {
                "name": "test-ns",
            }
        });

        let result = tracker.add(&gvr, &gvk, obj, "");
        assert!(result.is_ok());
    }

    #[test]
    fn test_add_allows_unknown_resource_any_namespace() {
        let tracker = ObjectTracker::default();
        let gvr = GVR::new("example.com", "v1", "widgets");
        let gvk = GVK::new("example.com", "v1", "Widget");
        let obj = json!({
            "apiVersion": "example.com/v1",
            "kind": "Widget",
            "metadata": {
                "name": "my-widget",
                "namespace": "default",
            }
        });

        // Unknown resources skip validation
        let result = tracker.add(&gvr, &gvk, obj, "default");
        assert!(result.is_ok());
    }

    #[test]
    fn test_cascade_cluster_to_cluster() {
        // Cluster-scoped owner -> cluster-scoped dependent: should cascade
        let tracker = ObjectTracker::default();
        let ns_gvr = GVR::new("", "v1", "namespaces");
        let ns_gvk = GVK::new("", "v1", "Namespace");

        // Create owner namespace (cluster-scoped, stored under "")
        let owner = json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": { "name": "owner-ns" }
        });
        let created_owner = tracker.add(&ns_gvr, &ns_gvk, owner, "").unwrap();
        let owner_uid = created_owner["metadata"]["uid"].as_str().unwrap();

        // Create dependent namespace (cluster-scoped) with ownerRef
        let mut dependent = json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": { "name": "child-ns" }
        });
        dependent["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "v1",
            "kind": "Namespace",
            "name": "owner-ns",
            "uid": owner_uid
        }]);
        tracker.add(&ns_gvr, &ns_gvk, dependent, "").unwrap();

        // Delete owner — should cascade to child
        tracker.delete(&ns_gvr, "", "owner-ns", true).unwrap();
        assert!(tracker.get(&ns_gvr, "", "owner-ns").is_err());
        assert!(tracker.get(&ns_gvr, "", "child-ns").is_err());
    }

    #[test]
    fn test_cascade_cluster_to_namespaced() {
        // Cluster-scoped owner -> namespaced dependent: should cascade
        let tracker = ObjectTracker::default();
        let ns_gvr = GVR::new("", "v1", "namespaces");
        let ns_gvk = GVK::new("", "v1", "Namespace");
        let pod_gvr = GVR::new("", "v1", "pods");
        let pod_gvk = GVK::new("", "v1", "Pod");

        // Create owner namespace (cluster-scoped)
        let owner = json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": { "name": "my-ns" }
        });
        let created_owner = tracker.add(&ns_gvr, &ns_gvk, owner, "").unwrap();
        let owner_uid = created_owner["metadata"]["uid"].as_str().unwrap();

        // Create dependent pod (namespaced) with ownerRef to the namespace
        let mut pod = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": { "name": "my-pod", "namespace": "my-ns" },
            "spec": { "containers": [{ "name": "c", "image": "nginx" }] }
        });
        pod["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "v1",
            "kind": "Namespace",
            "name": "my-ns",
            "uid": owner_uid
        }]);
        tracker.add(&pod_gvr, &pod_gvk, pod, "my-ns").unwrap();

        // Delete owner namespace — should cascade to the pod
        tracker.delete(&ns_gvr, "", "my-ns", true).unwrap();
        assert!(tracker.get(&ns_gvr, "", "my-ns").is_err());
        assert!(tracker.get(&pod_gvr, "my-ns", "my-pod").is_err());
    }

    #[test]
    fn test_cascade_namespaced_does_not_reach_cluster_scoped() {
        // Namespaced owner -> cluster-scoped dependent with matching UID: should NOT cascade
        let tracker = ObjectTracker::default();
        let pod_gvr = GVR::new("", "v1", "pods");
        let pod_gvk = GVK::new("", "v1", "Pod");
        let ns_gvr = GVR::new("", "v1", "namespaces");
        let ns_gvk = GVK::new("", "v1", "Namespace");

        // Create a namespaced pod as the owner
        let owner = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": { "name": "owner-pod", "namespace": "default" },
            "spec": { "containers": [{ "name": "c", "image": "nginx" }] }
        });
        let created_owner = tracker
            .create(&pod_gvr, &pod_gvk, owner, "default")
            .unwrap();
        let owner_uid = created_owner["metadata"]["uid"].as_str().unwrap();

        // Create a cluster-scoped namespace that (incorrectly) has an ownerRef to the pod
        // In real K8s this would be rejected, but we test that cascade doesn't cross the boundary
        let mut ns_obj = json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": { "name": "owned-ns" }
        });
        ns_obj["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "v1",
            "kind": "Pod",
            "name": "owner-pod",
            "uid": owner_uid
        }]);
        tracker.add(&ns_gvr, &ns_gvk, ns_obj, "").unwrap();

        // Delete the pod — should NOT cascade to the cluster-scoped namespace
        tracker
            .delete(&pod_gvr, "default", "owner-pod", true)
            .unwrap();
        assert!(tracker.get(&pod_gvr, "default", "owner-pod").is_err());
        // The namespace should survive — namespaced owner can't cascade to cluster-scoped
        assert!(tracker.get(&ns_gvr, "", "owned-ns").is_ok());
    }

    #[test]
    fn test_cascade_namespaced_does_not_reach_different_namespace() {
        // Namespaced owner -> namespaced dependent in different namespace: should NOT cascade
        let tracker = ObjectTracker::default();
        let pod_gvr = GVR::new("", "v1", "pods");
        let pod_gvk = GVK::new("", "v1", "Pod");

        // Create owner pod in namespace "ns-a"
        let owner = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": { "name": "owner-pod", "namespace": "ns-a" },
            "spec": { "containers": [{ "name": "c", "image": "nginx" }] }
        });
        let created_owner = tracker.create(&pod_gvr, &pod_gvk, owner, "ns-a").unwrap();
        let owner_uid = created_owner["metadata"]["uid"].as_str().unwrap();

        // Create dependent pod in namespace "ns-b" with ownerRef to owner
        let mut dependent = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": { "name": "cross-ns-pod", "namespace": "ns-b" },
            "spec": { "containers": [{ "name": "c", "image": "nginx" }] }
        });
        dependent["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "v1",
            "kind": "Pod",
            "name": "owner-pod",
            "uid": owner_uid
        }]);
        tracker
            .create(&pod_gvr, &pod_gvk, dependent, "ns-b")
            .unwrap();

        // Delete the owner — should NOT cascade to different-namespace dependent
        tracker.delete(&pod_gvr, "ns-a", "owner-pod", true).unwrap();
        assert!(tracker.get(&pod_gvr, "ns-a", "owner-pod").is_err());
        // The cross-namespace pod should survive
        assert!(tracker.get(&pod_gvr, "ns-b", "cross-ns-pod").is_ok());
    }
}
