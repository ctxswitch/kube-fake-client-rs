//! Tests for mock_service.rs functionality including:
//! - Patch type handling (JSON Patch, Merge Patch, Strategic Merge Patch, Apply Patch)
//! - Cluster-scoped resource support (Nodes, ClusterRoles, etc.)

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::ClientBuilder;
    use k8s_openapi::api::core::v1::{Node, Pod};
    use k8s_openapi::api::rbac::v1::ClusterRole;
    use kube::api::{DeleteParams, Patch, PatchParams, PostParams};
    use serde_json::json;

    // ============================================================================
    // Patch Type Tests
    // ============================================================================

    /// Test JSON Patch (RFC 6902) - application/json-patch+json
    /// JSON Patch uses an array of operations like add, remove, replace, etc.
    #[tokio::test]
    async fn test_json_patch_operations() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create a pod with some labels
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.labels = Some(
            [
                ("app".to_string(), "nginx".to_string()),
                ("env".to_string(), "dev".to_string()),
            ]
            .iter()
            .cloned()
            .collect(),
        );
        pods.create(&kube::api::PostParams::default(), &pod)
            .await
            .unwrap();

        // JSON Patch: add a new label, replace an existing one, and remove one
        let json_patch_value = json!([
            { "op": "add", "path": "/metadata/labels/version", "value": "1.0" },
            { "op": "replace", "path": "/metadata/labels/env", "value": "prod" },
            { "op": "remove", "path": "/metadata/labels/app" }
        ]);
        let json_patch: json_patch::Patch = serde_json::from_value(json_patch_value).unwrap();

        let patched: Pod = pods
            .patch(
                "test-pod",
                &PatchParams::default(),
                &Patch::<Pod>::Json(json_patch),
            )
            .await
            .unwrap();

        let labels = patched.metadata.labels.as_ref().unwrap();

        // Version should be added
        assert_eq!(labels.get("version").unwrap(), "1.0");

        // Env should be replaced
        assert_eq!(labels.get("env").unwrap(), "prod");

        // App should be removed
        assert!(!labels.contains_key("app"));
    }

    /// Test JSON Patch add operation on nested fields
    #[tokio::test]
    async fn test_json_patch_add_annotation() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create a pod without annotations
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pods.create(&kube::api::PostParams::default(), &pod)
            .await
            .unwrap();

        // Add annotations using JSON Patch
        let json_patch_value = json!([
            { "op": "add", "path": "/metadata/annotations", "value": { "note": "test annotation" } }
        ]);
        let json_patch: json_patch::Patch = serde_json::from_value(json_patch_value).unwrap();

        let patched: Pod = pods
            .patch(
                "test-pod",
                &PatchParams::default(),
                &Patch::<Pod>::Json(json_patch),
            )
            .await
            .unwrap();

        let annotations = patched.metadata.annotations.as_ref().unwrap();
        assert_eq!(annotations.get("note").unwrap(), "test annotation");
    }

    /// Test JSON Merge Patch (RFC 7386) - application/merge-patch+json
    /// Merge patch merges the provided JSON with the existing object
    #[tokio::test]
    async fn test_merge_patch() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create a pod with labels
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.labels = Some(
            [
                ("app".to_string(), "nginx".to_string()),
                ("env".to_string(), "dev".to_string()),
            ]
            .iter()
            .cloned()
            .collect(),
        );
        pods.create(&kube::api::PostParams::default(), &pod)
            .await
            .unwrap();

        // Merge patch: add new label and update existing
        let merge_patch = json!({
            "metadata": {
                "labels": {
                    "version": "2.0",
                    "env": "staging"
                }
            }
        });

        let patched: Pod = pods
            .patch(
                "test-pod",
                &PatchParams::default(),
                &Patch::Merge(&merge_patch),
            )
            .await
            .unwrap();

        let labels = patched.metadata.labels.as_ref().unwrap();

        // New label should be added
        assert_eq!(labels.get("version").unwrap(), "2.0");

        // Existing label should be updated
        assert_eq!(labels.get("env").unwrap(), "staging");

        // Original label should still exist (merge doesn't remove)
        assert_eq!(labels.get("app").unwrap(), "nginx");
    }

    /// Test Merge Patch with null values (removes fields)
    #[tokio::test]
    async fn test_merge_patch_with_null() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create a pod with labels and annotations
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.labels = Some(
            [("app".to_string(), "nginx".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        pod.metadata.annotations = Some(
            [("note".to_string(), "to be removed".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        pods.create(&kube::api::PostParams::default(), &pod)
            .await
            .unwrap();

        // Merge patch with null removes the field
        let merge_patch = json!({
            "metadata": {
                "annotations": null
            }
        });

        let patched: Pod = pods
            .patch(
                "test-pod",
                &PatchParams::default(),
                &Patch::Merge(&merge_patch),
            )
            .await
            .unwrap();

        // Annotations should be removed
        assert!(
            patched.metadata.annotations.is_none()
                || patched.metadata.annotations.as_ref().unwrap().is_empty()
        );

        // Labels should still exist
        assert!(patched.metadata.labels.is_some());
    }

    /// Test Strategic Merge Patch (Kubernetes-specific)
    /// Strategic merge is similar to merge but understands Kubernetes-specific semantics
    #[tokio::test]
    async fn test_strategic_merge_patch() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.labels = Some(
            [("app".to_string(), "nginx".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        pods.create(&kube::api::PostParams::default(), &pod)
            .await
            .unwrap();

        // Strategic merge patch
        let strategic_patch = json!({
            "metadata": {
                "labels": {
                    "tier": "backend"
                }
            }
        });

        let patched: Pod = pods
            .patch(
                "test-pod",
                &PatchParams::default(),
                &Patch::Strategic(&strategic_patch),
            )
            .await
            .unwrap();

        let labels = patched.metadata.labels.as_ref().unwrap();

        // Both labels should exist (strategic merge doesn't remove)
        assert_eq!(labels.get("app").unwrap(), "nginx");
        assert_eq!(labels.get("tier").unwrap(), "backend");
    }

    /// Test Apply Patch (Server-Side Apply)
    /// Apply patches are used for declarative configuration management
    #[tokio::test]
    async fn test_apply_patch() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.labels = Some(
            [("app".to_string(), "nginx".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        pods.create(&kube::api::PostParams::default(), &pod)
            .await
            .unwrap();

        // Apply patch (Server-Side Apply)
        let apply_patch = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "test-pod",
                "labels": {
                    "managed-by": "kubectl"
                }
            }
        });

        let patched: Pod = pods
            .patch(
                "test-pod",
                &PatchParams::apply("test-manager"),
                &Patch::Apply(&apply_patch),
            )
            .await
            .unwrap();

        let labels = patched.metadata.labels.as_ref().unwrap();

        // Original label should still exist
        assert_eq!(labels.get("app").unwrap(), "nginx");

        // Applied label should be added
        assert_eq!(labels.get("managed-by").unwrap(), "kubectl");
    }

    /// Test that different patch types behave differently
    #[tokio::test]
    async fn test_patch_type_differences() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Test 1: JSON Patch can remove fields
        let mut pod1 = Pod::default();
        pod1.metadata.name = Some("pod-json".to_string());
        pod1.metadata.labels = Some(
            [
                ("keep".to_string(), "yes".to_string()),
                ("remove".to_string(), "me".to_string()),
            ]
            .iter()
            .cloned()
            .collect(),
        );
        pods.create(&kube::api::PostParams::default(), &pod1)
            .await
            .unwrap();

        let json_patch_value = json!([
            { "op": "remove", "path": "/metadata/labels/remove" }
        ]);
        let json_patch: json_patch::Patch = serde_json::from_value(json_patch_value).unwrap();

        let patched1: Pod = pods
            .patch(
                "pod-json",
                &PatchParams::default(),
                &Patch::<Pod>::Json(json_patch),
            )
            .await
            .unwrap();

        let labels1 = patched1.metadata.labels.as_ref().unwrap();
        assert!(labels1.contains_key("keep"));
        assert!(!labels1.contains_key("remove")); // Removed by JSON Patch

        // Test 2: Merge Patch preserves unmentioned fields
        let mut pod2 = Pod::default();
        pod2.metadata.name = Some("pod-merge".to_string());
        pod2.metadata.labels = Some(
            [
                ("existing".to_string(), "label".to_string()),
                ("another".to_string(), "one".to_string()),
            ]
            .iter()
            .cloned()
            .collect(),
        );
        pods.create(&kube::api::PostParams::default(), &pod2)
            .await
            .unwrap();

        let merge_patch = json!({
            "metadata": {
                "labels": {
                    "new": "label"
                }
            }
        });

        let patched2: Pod = pods
            .patch(
                "pod-merge",
                &PatchParams::default(),
                &Patch::Merge(&merge_patch),
            )
            .await
            .unwrap();

        let labels2 = patched2.metadata.labels.as_ref().unwrap();
        assert_eq!(labels2.len(), 3); // All three labels should exist
        assert!(labels2.contains_key("existing"));
        assert!(labels2.contains_key("another"));
        assert!(labels2.contains_key("new"));
    }

    /// Test JSON Patch replace operation
    #[tokio::test]
    async fn test_json_patch_replace() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create a pod with a label
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.labels = Some(
            [("version".to_string(), "1.0".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        pods.create(&kube::api::PostParams::default(), &pod)
            .await
            .unwrap();

        // Replace the version label value
        let json_patch_value = json!([
            { "op": "replace", "path": "/metadata/labels/version", "value": "2.0" }
        ]);
        let json_patch: json_patch::Patch = serde_json::from_value(json_patch_value).unwrap();

        let patched: Pod = pods
            .patch(
                "test-pod",
                &PatchParams::default(),
                &Patch::<Pod>::Json(json_patch),
            )
            .await
            .unwrap();

        let labels = patched.metadata.labels.as_ref().unwrap();
        assert_eq!(labels.get("version").unwrap(), "2.0");
    }

    /// Test JSON Patch with multiple operations in sequence
    #[tokio::test]
    async fn test_json_patch_multiple_operations() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.labels = Some(
            [
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "2".to_string()),
            ]
            .iter()
            .cloned()
            .collect(),
        );
        pods.create(&kube::api::PostParams::default(), &pod)
            .await
            .unwrap();

        // Multiple operations: add, replace, remove
        let json_patch_value = json!([
            { "op": "add", "path": "/metadata/labels/c", "value": "3" },
            { "op": "replace", "path": "/metadata/labels/a", "value": "updated" },
            { "op": "remove", "path": "/metadata/labels/b" }
        ]);
        let json_patch: json_patch::Patch = serde_json::from_value(json_patch_value).unwrap();

        let patched: Pod = pods
            .patch(
                "test-pod",
                &PatchParams::default(),
                &Patch::<Pod>::Json(json_patch),
            )
            .await
            .unwrap();

        let labels = patched.metadata.labels.as_ref().unwrap();
        assert_eq!(labels.get("a").unwrap(), "updated"); // replaced
        assert!(!labels.contains_key("b")); // removed
        assert_eq!(labels.get("c").unwrap(), "3"); // added
    }

    /// Test that resource version is updated after patching
    #[tokio::test]
    async fn test_patch_updates_resource_version() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        let created = pods
            .create(&kube::api::PostParams::default(), &pod)
            .await
            .unwrap();

        let original_rv = created.metadata.resource_version.clone().unwrap();

        // Patch the pod
        let patch = json!({
            "metadata": {
                "labels": {
                    "patched": "true"
                }
            }
        });

        let patched: Pod = pods
            .patch("test-pod", &PatchParams::default(), &Patch::Merge(&patch))
            .await
            .unwrap();

        let new_rv = patched.metadata.resource_version.clone().unwrap();

        // Resource version should be incremented
        assert_ne!(original_rv, new_rv);
        assert!(new_rv.parse::<u64>().unwrap() > original_rv.parse::<u64>().unwrap());
    }

    // ============================================================================
    // Cluster-Scoped Resource Tests
    // ============================================================================

    /// Test creating and retrieving a cluster-scoped resource (Node)
    #[tokio::test]
    async fn test_cluster_scoped_node() {
        let client = ClientBuilder::new().build().await.unwrap();
        let nodes: kube::Api<Node> = kube::Api::all(client);

        // Create a node
        let mut node = Node::default();
        node.metadata.name = Some("node-1".to_string());

        let created = nodes.create(&PostParams::default(), &node).await.unwrap();

        // Verify it was created
        assert_eq!(created.metadata.name, Some("node-1".to_string()));
        assert_eq!(created.metadata.namespace, None); // Cluster-scoped resources have no namespace

        // Retrieve it
        let retrieved = nodes.get("node-1").await.unwrap();
        assert_eq!(retrieved.metadata.name, Some("node-1".to_string()));
        assert_eq!(retrieved.metadata.namespace, None);
    }

    /// Test creating and listing cluster-scoped resources
    #[tokio::test]
    async fn test_cluster_scoped_list() {
        let client = ClientBuilder::new().build().await.unwrap();
        let nodes: kube::Api<Node> = kube::Api::all(client);

        // Create multiple nodes
        for i in 1..=3 {
            let mut node = Node::default();
            node.metadata.name = Some(format!("node-{}", i));
            nodes.create(&PostParams::default(), &node).await.unwrap();
        }

        // List all nodes
        let node_list = nodes.list(&kube::api::ListParams::default()).await.unwrap();
        assert_eq!(node_list.items.len(), 3);

        // Verify all nodes have no namespace
        for node in &node_list.items {
            assert_eq!(node.metadata.namespace, None);
        }
    }

    /// Test cluster-scoped resource with API group (ClusterRole)
    #[tokio::test]
    async fn test_cluster_scoped_with_group() {
        let client = ClientBuilder::new().build().await.unwrap();
        let cluster_roles: kube::Api<ClusterRole> = kube::Api::all(client);

        // Create a ClusterRole
        let mut role = ClusterRole::default();
        role.metadata.name = Some("cluster-admin".to_string());
        role.rules = Some(vec![]);

        let created = cluster_roles
            .create(&PostParams::default(), &role)
            .await
            .unwrap();

        // Verify it was created
        assert_eq!(created.metadata.name, Some("cluster-admin".to_string()));
        assert_eq!(created.metadata.namespace, None);

        // Retrieve it
        let retrieved = cluster_roles.get("cluster-admin").await.unwrap();
        assert_eq!(retrieved.metadata.name, Some("cluster-admin".to_string()));
        assert_eq!(retrieved.metadata.namespace, None);
    }

    /// Test updating a cluster-scoped resource
    #[tokio::test]
    async fn test_cluster_scoped_update() {
        let client = ClientBuilder::new().build().await.unwrap();
        let nodes: kube::Api<Node> = kube::Api::all(client);

        // Create a node
        let mut node = Node::default();
        node.metadata.name = Some("node-1".to_string());
        node.metadata.labels = Some(
            [("region".to_string(), "us-west".to_string())]
                .iter()
                .cloned()
                .collect(),
        );

        let created = nodes.create(&PostParams::default(), &node).await.unwrap();

        // Update it
        let mut updated_node = created.clone();
        updated_node.metadata.labels = Some(
            [("region".to_string(), "us-east".to_string())]
                .iter()
                .cloned()
                .collect(),
        );

        let updated = nodes
            .replace("node-1", &PostParams::default(), &updated_node)
            .await
            .unwrap();

        // Verify the update
        assert_eq!(
            updated
                .metadata
                .labels
                .as_ref()
                .unwrap()
                .get("region")
                .unwrap(),
            "us-east"
        );
        assert_eq!(updated.metadata.namespace, None);
    }

    /// Test patching a cluster-scoped resource
    #[tokio::test]
    async fn test_cluster_scoped_patch() {
        let client = ClientBuilder::new().build().await.unwrap();
        let nodes: kube::Api<Node> = kube::Api::all(client);

        // Create a node
        let mut node = Node::default();
        node.metadata.name = Some("node-1".to_string());
        nodes.create(&PostParams::default(), &node).await.unwrap();

        // Patch it
        let patch = json!({
            "metadata": {
                "labels": {
                    "patched": "true"
                }
            }
        });

        let patched = nodes
            .patch(
                "node-1",
                &kube::api::PatchParams::default(),
                &kube::api::Patch::Merge(&patch),
            )
            .await
            .unwrap();

        // Verify the patch
        assert_eq!(
            patched
                .metadata
                .labels
                .as_ref()
                .unwrap()
                .get("patched")
                .unwrap(),
            "true"
        );
        assert_eq!(patched.metadata.namespace, None);
    }

    /// Test deleting a cluster-scoped resource
    #[tokio::test]
    async fn test_cluster_scoped_delete() {
        let client = ClientBuilder::new().build().await.unwrap();
        let nodes: kube::Api<Node> = kube::Api::all(client);

        // Create a node
        let mut node = Node::default();
        node.metadata.name = Some("node-1".to_string());
        nodes.create(&PostParams::default(), &node).await.unwrap();

        // Delete it
        nodes
            .delete("node-1", &kube::api::DeleteParams::default())
            .await
            .unwrap();

        // Verify it's gone
        let result = nodes.get("node-1").await;
        assert!(result.is_err());
    }

    /// Test that namespace is not set on cluster-scoped resources even if provided
    #[tokio::test]
    async fn test_cluster_scoped_ignores_namespace() {
        let client = ClientBuilder::new().build().await.unwrap();
        let nodes: kube::Api<Node> = kube::Api::all(client);

        // Try to create a node with a namespace set (should be ignored)
        let mut node = Node::default();
        node.metadata.name = Some("node-1".to_string());
        node.metadata.namespace = Some("should-be-ignored".to_string());

        let created = nodes.create(&PostParams::default(), &node).await.unwrap();

        // The namespace should NOT be set (cluster-scoped resources don't have namespaces)
        assert_eq!(created.metadata.namespace, None);
    }

    /// Test that single object delete only deletes the specified object, not a collection
    #[tokio::test]
    async fn test_single_delete_not_collection() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create multiple pods with the same labels
        for i in 1..=3 {
            let mut pod = Pod::default();
            pod.metadata.name = Some(format!("pod-{}", i));
            pod.metadata.labels = Some(
                [("app".to_string(), "nginx".to_string())]
                    .iter()
                    .cloned()
                    .collect(),
            );
            pods.create(&PostParams::default(), &pod).await.unwrap();
        }

        // Verify all 3 pods exist
        let list = pods.list(&kube::api::ListParams::default()).await.unwrap();
        assert_eq!(list.items.len(), 3);

        // Delete only pod-2 by name (single delete, not collection delete)
        pods.delete("pod-2", &kube::api::DeleteParams::default())
            .await
            .unwrap();

        // Verify only pod-2 was deleted and the others remain
        let list = pods.list(&kube::api::ListParams::default()).await.unwrap();
        assert_eq!(list.items.len(), 2);
        assert!(list
            .items
            .iter()
            .any(|p| p.metadata.name == Some("pod-1".to_string())));
        assert!(list
            .items
            .iter()
            .any(|p| p.metadata.name == Some("pod-3".to_string())));
        assert!(!list
            .items
            .iter()
            .any(|p| p.metadata.name == Some("pod-2".to_string())));
    }

    // ============================================================================
    // Field Selector Tests (through HTTP layer)
    // ============================================================================

    /// Test field selector metadata.name (universal field)
    #[tokio::test]
    async fn test_field_selector_metadata_name_http() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create multiple pods
        for i in 1..=3 {
            let mut pod = Pod::default();
            pod.metadata.name = Some(format!("pod-{}", i));
            pods.create(&PostParams::default(), &pod).await.unwrap();
        }

        // Filter by metadata.name
        let params = kube::api::ListParams::default().fields("metadata.name=pod-2");
        let filtered = pods.list(&params).await.unwrap();

        assert_eq!(filtered.items.len(), 1);
        assert_eq!(filtered.items[0].metadata.name, Some("pod-2".to_string()));
    }

    /// Test field selector metadata.namespace (universal field)
    #[tokio::test]
    async fn test_field_selector_metadata_namespace_http() {
        let client = ClientBuilder::new().build().await.unwrap();

        // Create pods in different namespaces
        let pods_default: kube::Api<Pod> = kube::Api::namespaced(client.clone(), "default");
        let pods_system: kube::Api<Pod> = kube::Api::namespaced(client.clone(), "kube-system");

        let mut pod1 = Pod::default();
        pod1.metadata.name = Some("pod-1".to_string());
        pods_default
            .create(&PostParams::default(), &pod1)
            .await
            .unwrap();

        let mut pod2 = Pod::default();
        pod2.metadata.name = Some("pod-2".to_string());
        pods_system
            .create(&PostParams::default(), &pod2)
            .await
            .unwrap();

        // List all pods with field selector for namespace
        let pods_all: kube::Api<Pod> = kube::Api::all(client);
        let params = kube::api::ListParams::default().fields("metadata.namespace=default");
        let filtered = pods_all.list(&params).await.unwrap();

        assert_eq!(filtered.items.len(), 1);
        assert_eq!(
            filtered.items[0].metadata.namespace,
            Some("default".to_string())
        );
    }

    /// Test field selector spec.nodeName (Pod-specific pre-registered field)
    #[tokio::test]
    async fn test_field_selector_spec_nodename_http() {
        use k8s_openapi::api::core::v1::{Container, PodSpec};

        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create pods with different node names
        let mut pod1 = Pod::default();
        pod1.metadata.name = Some("pod-1".to_string());
        pod1.spec = Some(PodSpec {
            node_name: Some("node-1".to_string()),
            containers: vec![Container {
                name: "app".to_string(),
                image: Some("app:latest".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        });
        pods.create(&PostParams::default(), &pod1).await.unwrap();

        let mut pod2 = Pod::default();
        pod2.metadata.name = Some("pod-2".to_string());
        pod2.spec = Some(PodSpec {
            node_name: Some("node-2".to_string()),
            containers: vec![Container {
                name: "app".to_string(),
                image: Some("app:latest".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        });
        pods.create(&PostParams::default(), &pod2).await.unwrap();

        // Filter by spec.nodeName
        let params = kube::api::ListParams::default().fields("spec.nodeName=node-1");
        let filtered = pods.list(&params).await.unwrap();

        assert_eq!(filtered.items.len(), 1);
        assert_eq!(filtered.items[0].metadata.name, Some("pod-1".to_string()));
        assert_eq!(
            filtered.items[0]
                .spec
                .as_ref()
                .and_then(|s| s.node_name.as_ref())
                .map(|s| s.as_str()),
            Some("node-1")
        );
    }

    /// Test field selector status.phase (Pod-specific pre-registered field)
    #[tokio::test]
    async fn test_field_selector_status_phase_http() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create pods and set their phases via status subresource
        let mut pod1 = Pod::default();
        pod1.metadata.name = Some("pod-1".to_string());
        pods.create(&PostParams::default(), &pod1).await.unwrap();

        let mut pod2 = Pod::default();
        pod2.metadata.name = Some("pod-2".to_string());
        pods.create(&PostParams::default(), &pod2).await.unwrap();

        // Update status to set phase
        let status_patch1 = json!({
            "status": {
                "phase": "Running"
            }
        });
        pods.patch_status(
            "pod-1",
            &kube::api::PatchParams::default(),
            &kube::api::Patch::Merge(&status_patch1),
        )
        .await
        .unwrap();

        let status_patch2 = json!({
            "status": {
                "phase": "Pending"
            }
        });
        pods.patch_status(
            "pod-2",
            &kube::api::PatchParams::default(),
            &kube::api::Patch::Merge(&status_patch2),
        )
        .await
        .unwrap();

        // Filter by status.phase
        let params = kube::api::ListParams::default().fields("status.phase=Running");
        let filtered = pods.list(&params).await.unwrap();

        assert_eq!(filtered.items.len(), 1);
        assert_eq!(filtered.items[0].metadata.name, Some("pod-1".to_string()));
        assert_eq!(
            filtered.items[0]
                .status
                .as_ref()
                .and_then(|s| s.phase.as_ref())
                .map(|s| s.as_str()),
            Some("Running")
        );
    }

    /// Test multiple field selectors combined
    #[tokio::test]
    async fn test_field_selector_multiple_fields_http() {
        use k8s_openapi::api::core::v1::{Container, PodSpec};

        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create multiple pods
        let mut pod1 = Pod::default();
        pod1.metadata.name = Some("target-pod".to_string());
        pod1.spec = Some(PodSpec {
            node_name: Some("node-1".to_string()),
            containers: vec![Container {
                name: "app".to_string(),
                image: Some("app:latest".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        });
        pods.create(&PostParams::default(), &pod1).await.unwrap();

        let mut pod2 = Pod::default();
        pod2.metadata.name = Some("target-pod-2".to_string());
        pod2.spec = Some(PodSpec {
            node_name: Some("node-2".to_string()),
            containers: vec![Container {
                name: "app".to_string(),
                image: Some("app:latest".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        });
        pods.create(&PostParams::default(), &pod2).await.unwrap();

        let mut pod3 = Pod::default();
        pod3.metadata.name = Some("other-pod".to_string());
        pod3.spec = Some(PodSpec {
            node_name: Some("node-1".to_string()),
            containers: vec![Container {
                name: "app".to_string(),
                image: Some("app:latest".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        });
        pods.create(&PostParams::default(), &pod3).await.unwrap();

        // Filter by both metadata.name AND spec.nodeName
        let params = kube::api::ListParams::default()
            .fields("metadata.name=target-pod,spec.nodeName=node-1");
        let filtered = pods.list(&params).await.unwrap();

        assert_eq!(filtered.items.len(), 1);
        assert_eq!(
            filtered.items[0].metadata.name,
            Some("target-pod".to_string())
        );
    }

    /// Test field selector with no matches
    #[tokio::test]
    async fn test_field_selector_no_match_http() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pods.create(&PostParams::default(), &pod).await.unwrap();

        // Filter by non-existent name
        let params = kube::api::ListParams::default().fields("metadata.name=nonexistent");
        let filtered = pods.list(&params).await.unwrap();

        assert_eq!(filtered.items.len(), 0);
    }

    /// Test field selector on cluster-scoped resources
    #[tokio::test]
    async fn test_field_selector_cluster_scoped_http() {
        let client = ClientBuilder::new().build().await.unwrap();
        let nodes: kube::Api<Node> = kube::Api::all(client);

        // Create nodes
        for i in 1..=3 {
            let mut node = Node::default();
            node.metadata.name = Some(format!("node-{}", i));
            nodes.create(&PostParams::default(), &node).await.unwrap();
        }

        // Filter by metadata.name (universal field)
        let params = kube::api::ListParams::default().fields("metadata.name=node-2");
        let filtered = nodes.list(&params).await.unwrap();

        assert_eq!(filtered.items.len(), 1);
        assert_eq!(filtered.items[0].metadata.name, Some("node-2".to_string()));
        assert_eq!(filtered.items[0].metadata.namespace, None);
    }

    // ============================================================================
    // DeleteCollection Tests
    // ============================================================================

    /// Test delete collection without selectors (deletes all)
    #[tokio::test]
    async fn test_delete_collection_all() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create multiple pods
        for i in 1..=3 {
            let mut pod = Pod::default();
            pod.metadata.name = Some(format!("pod-{}", i));
            pods.create(&PostParams::default(), &pod).await.unwrap();
        }

        // Verify they exist
        let list = pods.list(&kube::api::ListParams::default()).await.unwrap();
        assert_eq!(list.items.len(), 3);

        // Delete all pods
        pods.delete_collection(
            &kube::api::DeleteParams::default(),
            &kube::api::ListParams::default(),
        )
        .await
        .unwrap();

        // Verify they're all gone
        let list = pods.list(&kube::api::ListParams::default()).await.unwrap();
        assert_eq!(list.items.len(), 0);
    }

    /// Test delete collection with label selector
    #[tokio::test]
    async fn test_delete_collection_with_label_selector() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create pods with different labels
        for i in 1..=3 {
            let mut pod = Pod::default();
            pod.metadata.name = Some(format!("pod-{}", i));
            pod.metadata.labels = Some(
                [(
                    "app".to_string(),
                    if i <= 2 {
                        "nginx".to_string()
                    } else {
                        "redis".to_string()
                    },
                )]
                .iter()
                .cloned()
                .collect(),
            );
            pods.create(&PostParams::default(), &pod).await.unwrap();
        }

        // Delete only nginx pods
        let params = kube::api::ListParams::default().labels("app=nginx");
        pods.delete_collection(&kube::api::DeleteParams::default(), &params)
            .await
            .unwrap();

        // Verify only redis pod remains
        let list = pods.list(&kube::api::ListParams::default()).await.unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].metadata.name, Some("pod-3".to_string()));
    }

    /// Test delete collection with field selector
    #[tokio::test]
    async fn test_delete_collection_with_field_selector() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create multiple pods
        for i in 1..=3 {
            let mut pod = Pod::default();
            pod.metadata.name = Some(format!("pod-{}", i));
            pods.create(&PostParams::default(), &pod).await.unwrap();
        }

        // Delete specific pod by name using field selector
        let params = kube::api::ListParams::default().fields("metadata.name=pod-2");
        pods.delete_collection(&kube::api::DeleteParams::default(), &params)
            .await
            .unwrap();

        // Verify only pod-2 was deleted
        let list = pods.list(&kube::api::ListParams::default()).await.unwrap();
        assert_eq!(list.items.len(), 2);
        assert!(list
            .items
            .iter()
            .any(|p| p.metadata.name == Some("pod-1".to_string())));
        assert!(list
            .items
            .iter()
            .any(|p| p.metadata.name == Some("pod-3".to_string())));
    }

    /// Test delete collection with both label and field selectors
    #[tokio::test]
    async fn test_delete_collection_with_combined_selectors() {
        use k8s_openapi::api::core::v1::{Container, PodSpec};

        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create pods with labels and node assignments
        for i in 1..=4 {
            let mut pod = Pod::default();
            pod.metadata.name = Some(format!("pod-{}", i));
            pod.metadata.labels = Some(
                [(
                    "app".to_string(),
                    if i <= 2 {
                        "nginx".to_string()
                    } else {
                        "redis".to_string()
                    },
                )]
                .iter()
                .cloned()
                .collect(),
            );
            pod.spec = Some(PodSpec {
                node_name: Some(if i % 2 == 0 {
                    "node-1".to_string()
                } else {
                    "node-2".to_string()
                }),
                containers: vec![Container {
                    name: "app".to_string(),
                    image: Some("app:latest".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            });
            pods.create(&PostParams::default(), &pod).await.unwrap();
        }

        // Delete nginx pods on node-1 (should match only pod-2)
        let params = kube::api::ListParams::default()
            .labels("app=nginx")
            .fields("spec.nodeName=node-1");
        pods.delete_collection(&kube::api::DeleteParams::default(), &params)
            .await
            .unwrap();

        // Verify only pod-2 was deleted
        let list = pods.list(&kube::api::ListParams::default()).await.unwrap();
        assert_eq!(list.items.len(), 3);
        assert!(list
            .items
            .iter()
            .any(|p| p.metadata.name == Some("pod-1".to_string())));
        assert!(list
            .items
            .iter()
            .any(|p| p.metadata.name == Some("pod-3".to_string())));
        assert!(list
            .items
            .iter()
            .any(|p| p.metadata.name == Some("pod-4".to_string())));
    }

    /// Test delete collection on cluster-scoped resources
    #[tokio::test]
    async fn test_delete_collection_cluster_scoped() {
        let client = ClientBuilder::new().build().await.unwrap();
        let nodes: kube::Api<Node> = kube::Api::all(client);

        // Create nodes with labels
        for i in 1..=3 {
            let mut node = Node::default();
            node.metadata.name = Some(format!("node-{}", i));
            node.metadata.labels = Some(
                [(
                    "role".to_string(),
                    if i <= 2 {
                        "worker".to_string()
                    } else {
                        "master".to_string()
                    },
                )]
                .iter()
                .cloned()
                .collect(),
            );
            nodes.create(&PostParams::default(), &node).await.unwrap();
        }

        // Delete worker nodes
        let params = kube::api::ListParams::default().labels("role=worker");
        nodes
            .delete_collection(&kube::api::DeleteParams::default(), &params)
            .await
            .unwrap();

        // Verify only master node remains
        let list = nodes.list(&kube::api::ListParams::default()).await.unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].metadata.name, Some("node-3".to_string()));
    }

    /// Test delete collection with no matches
    #[tokio::test]
    async fn test_delete_collection_no_matches() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create a pod
        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.labels = Some(
            [("app".to_string(), "nginx".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        pods.create(&PostParams::default(), &pod).await.unwrap();

        // Try to delete with non-matching selector
        let params = kube::api::ListParams::default().labels("app=redis");
        pods.delete_collection(&kube::api::DeleteParams::default(), &params)
            .await
            .unwrap();

        // Verify pod still exists
        let list = pods.list(&kube::api::ListParams::default()).await.unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].metadata.name, Some("test-pod".to_string()));
    }

    // ============================================================================
    // Cascading Deletion via Owner References Tests
    // ============================================================================

    #[tokio::test]
    async fn test_delete_cascades_owner_references() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create the owner pod
        let mut owner = Pod::default();
        owner.metadata.name = Some("owner-pod".to_string());
        let created = pods.create(&PostParams::default(), &owner).await.unwrap();
        let owner_uid = created.metadata.uid.unwrap();

        // Create a child pod with ownerReferences pointing to the owner
        let mut child = Pod::default();
        child.metadata.name = Some("child-pod".to_string());
        child.metadata.owner_references = Some(vec![
            k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
                api_version: "v1".to_string(),
                kind: "Pod".to_string(),
                name: "owner-pod".to_string(),
                uid: owner_uid.clone(),
                ..Default::default()
            },
        ]);
        pods.create(&PostParams::default(), &child).await.unwrap();

        // Delete the owner
        pods.delete("owner-pod", &DeleteParams::default())
            .await
            .unwrap();

        // Child should be gone via cascading deletion
        assert!(pods.get("child-pod").await.is_err());
    }

    // ============================================================================
    // Server-Side Apply (SSA) Create-on-Patch Tests
    // ============================================================================

    /// SSA patch creates a new namespaced resource when it does not exist.
    #[tokio::test]
    async fn ssa_patch_creates_resource_when_not_found() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_body = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "ssa-pod"
            },
            "spec": {
                "containers": [{
                    "name": "nginx",
                    "image": "nginx:latest"
                }]
            }
        });

        let result = pods
            .patch(
                "ssa-pod",
                &PatchParams::apply("test-manager"),
                &Patch::Apply(&patch_body),
            )
            .await;
        assert!(result.is_ok(), "SSA patch should create the resource");

        let fetched = pods.get("ssa-pod").await.unwrap();
        assert_eq!(fetched.metadata.name.as_deref(), Some("ssa-pod"));
    }

    /// SSA patch updates an existing resource (normal patch behavior preserved).
    #[tokio::test]
    async fn ssa_patch_updates_existing_resource() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let mut pod = Pod::default();
        pod.metadata.name = Some("ssa-existing".to_string());
        pod.metadata.labels = Some(BTreeMap::from([("app".to_string(), "v1".to_string())]));
        pods.create(&PostParams::default(), &pod).await.unwrap();

        let patch_body = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "ssa-existing",
                "labels": {
                    "app": "v2"
                }
            }
        });

        let patched = pods
            .patch(
                "ssa-existing",
                &PatchParams::apply("test-manager"),
                &Patch::Apply(&patch_body),
            )
            .await
            .unwrap();

        let labels = patched.metadata.labels.unwrap();
        assert_eq!(labels.get("app").unwrap(), "v2");
    }

    /// SSA patch creates a cluster-scoped resource when it does not exist.
    #[tokio::test]
    async fn ssa_patch_creates_cluster_scoped_resource() {
        let client = ClientBuilder::new().build().await.unwrap();
        let nodes: kube::Api<Node> = kube::Api::all(client);

        let patch_body = json!({
            "apiVersion": "v1",
            "kind": "Node",
            "metadata": {
                "name": "ssa-node"
            }
        });

        let result = nodes
            .patch(
                "ssa-node",
                &PatchParams::apply("test-manager"),
                &Patch::Apply(&patch_body),
            )
            .await;
        assert!(
            result.is_ok(),
            "SSA patch should create a cluster-scoped resource"
        );

        let fetched = nodes.get("ssa-node").await.unwrap();
        assert_eq!(fetched.metadata.name.as_deref(), Some("ssa-node"));
    }

    /// Merge patch on a non-existent resource should still fail with not-found.
    #[tokio::test]
    async fn merge_patch_fails_when_resource_not_found() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_body = json!({
            "metadata": {
                "labels": { "app": "test" }
            }
        });

        let result = pods
            .patch(
                "nonexistent",
                &PatchParams::default(),
                &Patch::Merge(&patch_body),
            )
            .await;
        assert!(
            result.is_err(),
            "Merge patch should fail on non-existent resource"
        );
    }

    /// SSA patch creates a resource using the URL path name when the body omits metadata.name.
    #[tokio::test]
    async fn ssa_patch_creates_resource_without_name_in_body() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_body = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {},
            "spec": {
                "containers": [{
                    "name": "nginx",
                    "image": "nginx:latest"
                }]
            }
        });

        let result = pods
            .patch(
                "url-name-pod",
                &PatchParams::apply("test-manager"),
                &Patch::Apply(&patch_body),
            )
            .await;
        assert!(
            result.is_ok(),
            "SSA patch should use URL name when body omits it"
        );

        let fetched = pods.get("url-name-pod").await.unwrap();
        assert_eq!(fetched.metadata.name.as_deref(), Some("url-name-pod"));
        assert_eq!(
            fetched.metadata.namespace.as_deref(),
            Some("default"),
            "namespace should be set from URL path"
        );
    }

    /// SSA patch uses the URL path name even when the body contains a different name.
    #[tokio::test]
    async fn ssa_patch_url_name_overrides_body_name() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_body = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "body-name"
            }
        });

        let result = pods
            .patch(
                "url-name",
                &PatchParams::apply("test-manager"),
                &Patch::Apply(&patch_body),
            )
            .await
            .unwrap();

        assert_eq!(
            result.metadata.name.as_deref(),
            Some("url-name"),
            "URL path name must take precedence over body name"
        );
    }

    /// JSON patch on a non-existent resource should fail with not-found.
    #[tokio::test]
    async fn json_patch_fails_when_resource_not_found() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_body = json!([
            { "op": "add", "path": "/metadata/labels/app", "value": "test" }
        ]);

        let result = pods
            .patch(
                "nonexistent",
                &PatchParams::default(),
                &Patch::Json::<()>(serde_json::from_value(patch_body).unwrap()),
            )
            .await;
        assert!(
            result.is_err(),
            "JSON patch should fail on non-existent resource"
        );
    }

    // ============================================================================
    // parse_patch_params Tests
    // ============================================================================

    /// Verify that `parse_patch_params` extracts `fieldManager` and `force`.
    #[test]
    fn parse_patch_params_extracts_field_manager_and_force() {
        use crate::mock_service::MockService;

        let params = MockService::parse_patch_params(Some("fieldManager=test-mgr&force=true"));
        assert_eq!(params.field_manager.as_deref(), Some("test-mgr"));
        assert!(params.force);
    }

    /// Verify that `parse_patch_params` handles URL-encoded `fieldManager`.
    #[test]
    fn parse_patch_params_decodes_url_encoded_field_manager() {
        use crate::mock_service::MockService;

        let params = MockService::parse_patch_params(Some("fieldManager=my%20manager"));
        assert_eq!(params.field_manager.as_deref(), Some("my manager"));
    }

    /// Verify that `parse_patch_params` defaults to `force=false`.
    #[test]
    fn parse_patch_params_defaults_force_to_false() {
        use crate::mock_service::MockService;

        let params = MockService::parse_patch_params(Some("fieldManager=mgr"));
        assert!(!params.force);
    }

    /// Verify that `parse_patch_params` treats `force=false` as false.
    #[test]
    fn parse_patch_params_force_false() {
        use crate::mock_service::MockService;

        let params = MockService::parse_patch_params(Some("force=false"));
        assert!(!params.force);
    }

    /// Verify that `parse_patch_params` handles bare `force` (no value).
    #[test]
    fn parse_patch_params_bare_force() {
        use crate::mock_service::MockService;

        let params = MockService::parse_patch_params(Some("force"));
        assert!(params.force);
    }

    /// Verify that `parse_patch_params` returns defaults for `None` query.
    #[test]
    fn parse_patch_params_none_query() {
        use crate::mock_service::MockService;

        let params = MockService::parse_patch_params(None);
        assert_eq!(params.field_manager, None);
        assert!(!params.force);
    }

    /// Verify that `parse_patch_params` returns defaults for empty query string.
    #[test]
    fn parse_patch_params_empty_query() {
        use crate::mock_service::MockService;

        let params = MockService::parse_patch_params(Some(""));
        assert_eq!(params.field_manager, None);
        assert!(!params.force);
    }

    /// SSA (apply patch) without `fieldManager` returns 422.
    #[tokio::test]
    async fn ssa_patch_without_field_manager_returns_422() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_body = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "no-fm-pod"
            }
        });

        // PatchParams::default() has no fieldManager set
        let result = pods
            .patch(
                "no-fm-pod",
                &PatchParams::default(),
                &Patch::Apply(&patch_body),
            )
            .await;

        let err = result.unwrap_err();
        if let kube::Error::Api(api_err) = &err {
            assert_eq!(api_err.code, 422, "expected 422, got {}", api_err.code);
            assert!(
                api_err.message.contains("fieldManager"),
                "error message should mention fieldManager: {}",
                api_err.message
            );
        } else {
            panic!("expected kube::Error::Api, got: {err:?}");
        }
    }

    /// SSA patch on /status subresource should still fail when resource does not exist.
    #[tokio::test]
    async fn ssa_status_patch_fails_when_resource_not_found() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_body = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "status": {
                "phase": "Running"
            }
        });

        let result = pods
            .patch_status(
                "nonexistent",
                &PatchParams::apply("test-manager"),
                &Patch::Apply(&patch_body),
            )
            .await;
        assert!(
            result.is_err(),
            "SSA status patch should not create a resource"
        );
    }

    // ============================================================================
    // Field Ownership / managedFields Integration Tests
    // ============================================================================

    /// SSA apply on a non-existent pod creates it with managedFields populated.
    #[tokio::test]
    async fn ssa_creates_object_with_managed_fields() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_body = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "mf-pod",
                "labels": { "app": "web" }
            }
        });

        let pod = pods
            .patch(
                "mf-pod",
                &PatchParams::apply("test-mgr"),
                &Patch::Apply(&patch_body),
            )
            .await
            .unwrap();

        let mf = pod.metadata.managed_fields.unwrap_or_default();
        assert_eq!(mf.len(), 1, "should have exactly one managedFields entry");
        assert_eq!(
            mf[0].manager.as_deref(),
            Some("test-mgr"),
            "manager should match"
        );
    }

    /// Two different field managers apply non-overlapping fields. Both succeed
    /// and managedFields has 2 entries.
    #[tokio::test]
    async fn ssa_two_managers_different_fields_no_conflict() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_a = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "mf-pod2",
                "labels": { "app": "web" }
            }
        });

        pods.patch(
            "mf-pod2",
            &PatchParams::apply("manager-a"),
            &Patch::Apply(&patch_a),
        )
        .await
        .unwrap();

        let patch_b = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "mf-pod2",
                "annotations": { "note": "hello" }
            }
        });

        let pod = pods
            .patch(
                "mf-pod2",
                &PatchParams::apply("manager-b"),
                &Patch::Apply(&patch_b),
            )
            .await
            .unwrap();

        let mf = pod.metadata.managed_fields.unwrap_or_default();
        assert_eq!(mf.len(), 2, "should have 2 managedFields entries");

        let managers: Vec<&str> = mf.iter().filter_map(|e| e.manager.as_deref()).collect();
        assert!(managers.contains(&"manager-a"));
        assert!(managers.contains(&"manager-b"));
    }

    /// Two managers apply overlapping fields with force=false. Second apply
    /// returns 409 Conflict.
    #[tokio::test]
    async fn ssa_two_managers_conflict_returns_409() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_a = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "conflict-pod",
                "labels": { "app": "web" }
            }
        });

        pods.patch(
            "conflict-pod",
            &PatchParams::apply("manager-a"),
            &Patch::Apply(&patch_a),
        )
        .await
        .unwrap();

        // Second manager tries to own the same label field
        let patch_b = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "conflict-pod",
                "labels": { "app": "other" }
            }
        });

        let result = pods
            .patch(
                "conflict-pod",
                &PatchParams::apply("manager-b"),
                &Patch::Apply(&patch_b),
            )
            .await;

        let err = result.unwrap_err();
        match &err {
            kube::Error::Api(ref api_err) if api_err.code == 409 => {}
            other => panic!("expected 409 Conflict, got: {other:?}"),
        }
    }

    /// Same overlapping fields but second manager uses force=true. Succeeds and
    /// first manager's conflicting fields are pruned.
    #[tokio::test]
    async fn ssa_force_true_takes_ownership() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_a = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "force-pod",
                "labels": { "app": "web" }
            }
        });

        pods.patch(
            "force-pod",
            &PatchParams::apply("manager-a"),
            &Patch::Apply(&patch_a),
        )
        .await
        .unwrap();

        let patch_b = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "force-pod",
                "labels": { "app": "taken" }
            }
        });

        let mut pp = PatchParams::apply("manager-b");
        pp.force = true;

        let pod = pods
            .patch("force-pod", &pp, &Patch::Apply(&patch_b))
            .await
            .unwrap();

        let mf = pod.metadata.managed_fields.unwrap_or_default();
        // manager-a should have been fully pruned (only had that one label)
        let managers: Vec<&str> = mf.iter().filter_map(|e| e.manager.as_deref()).collect();
        assert!(
            managers.contains(&"manager-b"),
            "manager-b should own the field"
        );
        // manager-a either absent or has no overlapping fields
        assert!(
            !managers.contains(&"manager-a"),
            "manager-a should have been pruned"
        );
    }

    /// Verify the shape of a managedFields entry.
    #[tokio::test]
    async fn ssa_managed_fields_entry_shape() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let patch_body = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "shape-pod",
                "labels": { "app": "web" }
            }
        });

        pods.patch(
            "shape-pod",
            &PatchParams::apply("shape-mgr"),
            &Patch::Apply(&patch_body),
        )
        .await
        .unwrap();

        // GET the pod back as raw JSON to inspect the full entry shape
        let raw_pod = pods.get("shape-pod").await.unwrap();
        let mf = raw_pod.metadata.managed_fields.unwrap_or_default();
        assert!(!mf.is_empty(), "managedFields should not be empty");

        let entry = &mf[0];
        assert_eq!(entry.manager.as_deref(), Some("shape-mgr"));
        assert_eq!(entry.operation.as_deref(), Some("Apply"));
        assert_eq!(entry.api_version.as_deref(), Some("v1"));
        assert_eq!(entry.fields_type.as_deref(), Some("FieldsV1"));
        assert!(entry.fields_v1.is_some(), "fieldsV1 should be present");
        assert!(entry.time.is_some(), "time should be present");
        assert!(
            entry.subresource.is_none() || entry.subresource.as_deref() == Some(""),
            "subresource should not be set for main resource"
        );
    }

    /// SSA on /status produces an entry with subresource: "status" and doesn't
    /// conflict with main resource manager owning the same fields.
    #[tokio::test]
    async fn ssa_status_subresource_uses_status_subresource() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // First create the pod via SSA on main resource
        let create_patch = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "status-sub-pod",
                "labels": { "app": "web" }
            }
        });

        pods.patch(
            "status-sub-pod",
            &PatchParams::apply("main-mgr"),
            &Patch::Apply(&create_patch),
        )
        .await
        .unwrap();

        // Now SSA on /status with the same manager name — should not conflict
        let status_patch = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "status": {
                "phase": "Running"
            }
        });

        let pod = pods
            .patch_status(
                "status-sub-pod",
                &PatchParams::apply("main-mgr"),
                &Patch::Apply(&status_patch),
            )
            .await
            .unwrap();

        let mf = pod.metadata.managed_fields.unwrap_or_default();
        assert!(
            mf.len() >= 2,
            "should have at least 2 entries (main + status)"
        );

        let status_entries: Vec<_> = mf
            .iter()
            .filter(|e| e.subresource.as_deref() == Some("status"))
            .collect();
        assert_eq!(
            status_entries.len(),
            1,
            "should have exactly one status subresource entry"
        );
        assert_eq!(status_entries[0].manager.as_deref(), Some("main-mgr"));
    }

    /// A merge patch doesn't add managedFields to the object.
    #[tokio::test]
    async fn client_side_patches_dont_touch_managed_fields() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        // Create the pod first
        let mut pod = Pod::default();
        pod.metadata.name = Some("merge-pod".to_string());
        pods.create(&PostParams::default(), &pod).await.unwrap();

        // Apply a merge patch
        let merge_body = json!({
            "metadata": {
                "labels": { "patched": "yes" }
            }
        });

        let patched = pods
            .patch(
                "merge-pod",
                &PatchParams::default(),
                &Patch::Merge(&merge_body),
            )
            .await
            .unwrap();

        assert!(
            patched.metadata.managed_fields.is_none()
                || patched.metadata.managed_fields.as_ref().unwrap().is_empty(),
            "merge patch should not create managedFields"
        );
    }
}
