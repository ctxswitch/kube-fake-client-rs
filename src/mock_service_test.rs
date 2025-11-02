//! Tests for mock_service.rs functionality including:
//! - Patch type handling (JSON Patch, Merge Patch, Strategic Merge Patch, Apply Patch)
//! - Cluster-scoped resource support (Nodes, ClusterRoles, etc.)

#[cfg(test)]
mod tests {
    use crate::ClientBuilder;
    use k8s_openapi::api::core::v1::{Node, Pod};
    use k8s_openapi::api::rbac::v1::ClusterRole;
    use kube::api::{Patch, PatchParams, PostParams};
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
}
