//! Tests for mock_service.rs functionality including:
//! - Patch type handling (JSON Patch, Merge Patch, Strategic Merge Patch, Apply Patch)
//! - Cluster-scoped resource support (Nodes, ClusterRoles, etc.)

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::ClientBuilder;
    use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec, ReplicaSetSpec, StatefulSetSpec};
    use k8s_openapi::api::batch::v1::Job;
    use k8s_openapi::api::core::v1::{ConfigMap, Namespace, Node, Pod, ReplicationController};
    use k8s_openapi::api::rbac::v1::ClusterRole;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ManagedFieldsEntry;
    use kube::api::{
        DeleteParams, EvictParams, ListParams, LogParams, Patch, PatchParams, PostParams,
    };
    use kube::core::NamespaceResourceScope;
    use kube::Resource;
    use serde::de::DeserializeOwned;
    use serde::Serialize;
    use serde_json::json;
    use std::fmt::Debug;

    /// Returns a `spec`, which some kinds stopped leaving optional in v1_37.
    fn spec_of<S>(spec: impl Into<Option<S>>) -> S {
        spec.into().unwrap()
    }

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
    // Namespace Resource Tests
    // ============================================================================

    /// Namespace supports CRUD and the status subresource through `Api::all`.
    #[tokio::test]
    async fn test_namespace_resource_crud_and_status() {
        let client = ClientBuilder::new().build().await.unwrap();
        let api: kube::Api<Namespace> = kube::Api::all(client);
        let pp = PostParams::default();

        let mut ns = Namespace::default();
        ns.metadata.name = Some("team-a".to_string());
        let created = api.create(&pp, &ns).await.unwrap();
        assert_eq!(created.metadata.namespace, None);
        ns.metadata.name = Some("team-b".to_string());
        api.create(&pp, &ns).await.unwrap();

        let fetched = api.get("team-a").await.unwrap();
        assert_eq!(fetched.metadata.name.as_deref(), Some("team-a"));
        assert_eq!(api.list(&Default::default()).await.unwrap().items.len(), 2);

        let mut updated = fetched;
        updated.metadata.labels = Some(BTreeMap::from([("env".to_string(), "dev".to_string())]));
        api.replace("team-a", &pp, &updated).await.unwrap();
        let fetched = api.get("team-a").await.unwrap();
        assert_eq!(
            fetched.metadata.labels.as_ref().and_then(|l| l.get("env")),
            Some(&"dev".to_string())
        );

        api.patch(
            "team-a",
            &PatchParams::default(),
            &Patch::Merge(json!({"metadata": {"labels": {"tier": "gold"}}})),
        )
        .await
        .unwrap();
        let fetched = api.get("team-a").await.unwrap();
        assert_eq!(
            fetched.metadata.labels.as_ref().and_then(|l| l.get("tier")),
            Some(&"gold".to_string())
        );

        let mut with_status = fetched;
        with_status.status = Some(Default::default());
        with_status.status.as_mut().unwrap().phase = Some("Active".to_string());
        api.replace_status("team-a", &pp, &with_status)
            .await
            .unwrap();
        let status = api.get_status("team-a").await.unwrap();
        assert_eq!(
            status.status.and_then(|s| s.phase).as_deref(),
            Some("Active")
        );

        api.patch_status(
            "team-a",
            &PatchParams::default(),
            &Patch::Merge(json!({"status": {"conditions": [{"type": "Ready", "status": "True"}]}})),
        )
        .await
        .unwrap();
        let status = api.get_status("team-a").await.unwrap();
        let status = status.status.unwrap();
        assert_eq!(status.phase.as_deref(), Some("Active"));
        let conditions = status.conditions.unwrap();
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].type_, "Ready");

        api.delete("team-b", &DeleteParams::default())
            .await
            .unwrap();
        match api.get("team-b").await {
            Err(kube::Error::Api(err)) => {
                assert_eq!(err.code, 404);
                assert_eq!(err.reason, "NotFound");
            }
            other => panic!("expected 404 NotFound, got {other:?}"),
        }
    }

    /// A Namespace named `status` does not change how paths inside it are parsed.
    #[tokio::test]
    async fn test_namespace_resource_named_status_does_not_shadow_namespaced_paths() {
        let client = ClientBuilder::new().build().await.unwrap();
        let namespaces: kube::Api<Namespace> = kube::Api::all(client.clone());
        let mut ns = Namespace::default();
        ns.metadata.name = Some("status".to_string());
        namespaces
            .create(&PostParams::default(), &ns)
            .await
            .unwrap();
        namespaces.get("status").await.unwrap();
        namespaces.get_status("status").await.unwrap();

        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "status");
        let mut pod = Pod::default();
        pod.metadata.name = Some("p".to_string());
        pods.create(&PostParams::default(), &pod).await.unwrap();
        pods.get("p").await.unwrap();
        assert_eq!(pods.list(&Default::default()).await.unwrap().items.len(), 1);
    }

    /// Namespace `finalize` is not implemented and returns 404.
    #[tokio::test]
    async fn test_namespace_resource_finalize_returns_404() {
        let client = ClientBuilder::new().build().await.unwrap();
        let api: kube::Api<Namespace> = kube::Api::all(client);
        let mut ns = Namespace::default();
        ns.metadata.name = Some("team-a".to_string());
        api.create(&PostParams::default(), &ns).await.unwrap();

        let result = api.get_subresource("finalize", "team-a").await;
        assert_subresource_not_found(result, "namespaces/finalize \"team-a\"");
        let result = api
            .replace_subresource("finalize", "team-a", &PostParams::default(), &ns)
            .await;
        assert_subresource_not_found(result, "namespaces/finalize \"team-a\"");
    }

    /// Paths that do not parse as API resource paths return a 404 `Status`.
    #[tokio::test]
    async fn test_unmatched_path_returns_404_status() {
        let client = ClientBuilder::new().build().await.unwrap();
        let cases = [
            ("GET", "/api"),
            ("GET", "/api/v1"),
            ("GET", "/apis/apps/v1/namespaces"),
            ("PUT", "/apis"),
            ("DELETE", "/version"),
        ];
        for (method, uri) in cases {
            let req = http::Request::builder()
                .method(method)
                .uri(uri)
                .body(Vec::new())
                .unwrap();
            match client.request_text(req).await {
                Err(kube::Error::Api(s)) => {
                    assert_eq!(s.code, 404, "{method} {uri}");
                    assert_eq!(s.reason, "NotFound", "{method} {uri}");
                    assert_eq!(
                        s.message, "the server could not find the requested resource",
                        "{method} {uri}"
                    );
                }
                other => panic!("{method} {uri}: expected kube::Error::Api, got {other:?}"),
            }
        }
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

    /// Delete of an object with finalizers keeps it until the last finalizer is removed.
    #[tokio::test]
    async fn test_delete_with_finalizers_waits_for_finalizers() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let mut pod = Pod::default();
        pod.metadata.name = Some("test-pod".to_string());
        pod.metadata.finalizers = Some(vec!["example.com/cleanup".to_string()]);
        pods.create(&PostParams::default(), &pod).await.unwrap();

        pods.delete("test-pod", &DeleteParams::default())
            .await
            .unwrap();
        let terminating = pods.get("test-pod").await.unwrap();
        assert!(terminating.metadata.deletion_timestamp.is_some());

        pods.patch(
            "test-pod",
            &PatchParams::default(),
            &Patch::Merge(json!({ "metadata": { "finalizers": null } })),
        )
        .await
        .unwrap();
        assert!(pods.get("test-pod").await.is_err());
    }

    /// Deleting a Namespace deletes its contents and waits for their finalizers.
    #[tokio::test]
    async fn test_delete_namespace_deletes_contents_and_waits_for_finalizers() {
        use k8s_openapi::api::core::v1::ConfigMap;

        let client = ClientBuilder::new().build().await.unwrap();
        let namespaces: kube::Api<Namespace> = kube::Api::all(client.clone());
        let configmaps: kube::Api<ConfigMap> = kube::Api::namespaced(client, "team-a");
        let pp = PostParams::default();

        let mut ns = Namespace::default();
        ns.metadata.name = Some("team-a".to_string());
        namespaces.create(&pp, &ns).await.unwrap();
        let mut plain = ConfigMap::default();
        plain.metadata.name = Some("plain".to_string());
        configmaps.create(&pp, &plain).await.unwrap();
        let mut held = ConfigMap::default();
        held.metadata.name = Some("held".to_string());
        held.metadata.finalizers = Some(vec!["example.com/cleanup".to_string()]);
        configmaps.create(&pp, &held).await.unwrap();

        namespaces
            .delete("team-a", &DeleteParams::default())
            .await
            .unwrap();

        let terminating = namespaces.get("team-a").await.unwrap();
        assert!(terminating.metadata.deletion_timestamp.is_some());
        assert_eq!(
            terminating.status.and_then(|s| s.phase).as_deref(),
            Some("Terminating")
        );
        assert!(configmaps.get("plain").await.is_err());
        let held = configmaps.get("held").await.unwrap();
        assert!(held.metadata.deletion_timestamp.is_some());

        // No new content in a terminating namespace
        let mut new = ConfigMap::default();
        new.metadata.name = Some("new".to_string());
        match configmaps.create(&pp, &new).await {
            Err(kube::Error::Api(err)) => {
                assert_eq!(err.code, 403);
                assert_eq!(err.reason, "Forbidden");
                assert_eq!(
                    err.message,
                    "configmaps \"new\" is forbidden: unable to create new content in namespace team-a because it is being terminated"
                );
                let details = err.details.expect("forbidden status has details");
                assert_eq!(details.kind, "configmaps");
                assert_eq!(details.name, "new");
                assert_eq!(details.causes.len(), 1);
                assert_eq!(details.causes[0].reason, "NamespaceTerminating");
                assert_eq!(details.causes[0].field, "metadata.namespace");
            }
            other => panic!("expected 403 Forbidden, got {other:?}"),
        }
        assert!(configmaps.get("new").await.is_err());

        // Existing content can still drop its finalizers
        configmaps
            .patch(
                "held",
                &PatchParams::default(),
                &Patch::Merge(json!({ "metadata": { "finalizers": null } })),
            )
            .await
            .unwrap();
        assert!(configmaps.get("held").await.is_err());
        assert!(namespaces.get("team-a").await.is_err());
    }

    /// A Deployment gets the spec/status split without `with_status_subresource`.
    #[tokio::test]
    async fn test_builtin_status_subresource_needs_no_configuration() {
        use k8s_openapi::api::apps::v1::DeploymentStatus;

        let client = ClientBuilder::new().build().await.unwrap();
        let deployments: kube::Api<Deployment> = kube::Api::namespaced(client, "default");
        let pp = PostParams::default();

        let deployment: Deployment = serde_json::from_value(json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": { "name": "web" },
            "spec": { "replicas": 1 },
        }))
        .unwrap();
        deployments.create(&pp, &deployment).await.unwrap();

        let mut current = deployments.get("web").await.unwrap();
        current.status = Some(DeploymentStatus {
            replicas: Some(5),
            ..Default::default()
        });
        deployments.replace("web", &pp, &current).await.unwrap();
        assert!(deployments.get("web").await.unwrap().status.is_none());

        deployments
            .patch_status(
                "web",
                &PatchParams::default(),
                &Patch::Merge(json!({ "spec": { "replicas": 9 }, "status": { "replicas": 1 } })),
            )
            .await
            .unwrap();
        let current = deployments.get("web").await.unwrap();
        assert_eq!(spec_of::<DeploymentSpec>(current.spec).replicas, Some(1));
        assert_eq!(current.status.and_then(|s| s.replicas), Some(1));
    }

    /// A kind without a status subresource has no `/status` path.
    #[tokio::test]
    async fn test_status_on_kind_without_status_subresource_returns_404() {
        use k8s_openapi::api::core::v1::ConfigMap;

        let client = ClientBuilder::new().build().await.unwrap();
        let configmaps: kube::Api<ConfigMap> = kube::Api::namespaced(client.clone(), "default");
        let mut cm = ConfigMap::default();
        cm.metadata.name = Some("cm".to_string());
        cm.data = Some(BTreeMap::from([("key".to_string(), "value".to_string())]));
        let created = configmaps
            .create(&PostParams::default(), &cm)
            .await
            .unwrap();

        let needle = "configmaps/status \"cm\"";
        assert_subresource_not_found(configmaps.get_status("cm").await, needle);
        assert_subresource_not_found(
            configmaps
                .replace_status("cm", &PostParams::default(), &created)
                .await,
            needle,
        );
        assert_subresource_not_found(
            configmaps
                .patch_status(
                    "cm",
                    &PatchParams::default(),
                    &Patch::Merge(json!({ "data": { "key": "changed" } })),
                )
                .await,
            needle,
        );
        let post = http::Request::post("/api/v1/namespaces/default/configmaps/cm/status")
            .body(Vec::new())
            .unwrap();
        assert_subresource_not_found(client.request_text(post).await, needle);
        let delete = http::Request::delete("/api/v1/namespaces/default/configmaps/cm/status")
            .body(Vec::new())
            .unwrap();
        assert_subresource_not_found(client.request_text(delete).await, needle);
        assert_eq!(configmaps.get("cm").await.unwrap(), created);
    }

    fn web_deployment() -> k8s_openapi::api::apps::v1::Deployment {
        serde_json::from_value(json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": { "name": "web", "labels": { "team": "a" } },
            "spec": {
                "replicas": 1,
                "selector": { "matchLabels": { "app": "web" } },
                "template": {
                    "metadata": { "labels": { "app": "web" } },
                    "spec": { "containers": [{ "name": "web", "image": "nginx" }] }
                }
            }
        }))
        .unwrap()
    }

    /// A write that changes nothing keeps `resourceVersion` on every write path,
    /// and the next real write gets the next one.
    #[tokio::test]
    async fn test_noop_writes_keep_resource_version() {
        use k8s_openapi::api::apps::v1::Deployment;

        let client = ClientBuilder::new().build().await.unwrap();
        let deployments: kube::Api<Deployment> = kube::Api::namespaced(client, "default");
        let params = PatchParams::default();
        deployments
            .create(&PostParams::default(), &web_deployment())
            .await
            .unwrap();
        let status = Patch::Merge(json!({ "status": { "replicas": 1 } }));
        let patched = deployments
            .patch_status("web", &params, &status)
            .await
            .unwrap();
        let version = patched.metadata.resource_version;

        // Built from scratch: no namespace, uid, resourceVersion or status.
        let replaced = deployments
            .replace("web", &PostParams::default(), &web_deployment())
            .await
            .unwrap();
        assert_eq!(replaced.metadata.resource_version, version);
        assert_eq!(replaced.metadata.namespace.as_deref(), Some("default"));

        let merged = deployments
            .patch(
                "web",
                &params,
                &Patch::Merge(json!({ "spec": { "replicas": 1 } })),
            )
            .await
            .unwrap();
        assert_eq!(merged.metadata.resource_version, version);

        let status_patched = deployments
            .patch_status("web", &params, &status)
            .await
            .unwrap();
        assert_eq!(status_patched.metadata.resource_version, version);

        let scaled = deployments
            .patch_scale(
                "web",
                &params,
                &Patch::Merge(json!({ "spec": { "replicas": 1 } })),
            )
            .await
            .unwrap();
        assert_eq!(scaled.metadata.resource_version, version);

        let written = deployments
            .patch(
                "web",
                &params,
                &Patch::Merge(json!({ "metadata": { "labels": { "team": "b" } } })),
            )
            .await
            .unwrap();
        let next = version.unwrap().parse::<u64>().unwrap() + 1;
        assert_eq!(written.metadata.resource_version, Some(next.to_string()));
    }

    /// The conflict check runs before a write is found to change nothing.
    #[tokio::test]
    async fn test_noop_replace_with_stale_resource_version_conflicts() {
        use k8s_openapi::api::apps::v1::Deployment;

        let client = ClientBuilder::new().build().await.unwrap();
        let deployments: kube::Api<Deployment> = kube::Api::namespaced(client, "default");
        let mut stale = deployments
            .create(&PostParams::default(), &web_deployment())
            .await
            .unwrap();
        stale.metadata.resource_version = Some("999".to_string());

        let error = deployments
            .replace("web", &PostParams::default(), &stale)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, kube::Error::Api(status) if status.code == 409),
            "{error:?}"
        );
    }

    const SEEDED_TIME: &str = "2020-01-01T00:00:00Z";

    fn config_map_manifest(data: serde_json::Value) -> serde_json::Value {
        json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": { "name": "applied", "namespace": "default" },
            "data": data,
        })
    }

    /// The entry an apply of `manifest` records, stamped in the past so a test
    /// can tell it from one written now.
    fn seeded_entry(manager: &str, manifest: &serde_json::Value) -> serde_json::Value {
        json!({
            "manager": manager,
            "operation": "Apply",
            "apiVersion": "v1",
            "fieldsType": "FieldsV1",
            "fieldsV1": crate::fields_v1::build_fields_v1(manifest),
            "time": SEEDED_TIME,
        })
    }

    /// A client holding a ConfigMap with `data` and the given `managedFields`.
    async fn seeded_config_maps(
        data: serde_json::Value,
        entries: Vec<serde_json::Value>,
    ) -> kube::Api<ConfigMap> {
        let mut seeded = config_map_manifest(data);
        seeded["metadata"]["managedFields"] = json!(entries);
        let client = ClientBuilder::new()
            .with_runtime_objects(vec![seeded])
            .build()
            .await
            .unwrap();
        kube::Api::namespaced(client, "default")
    }

    fn entry_times(config_map: &ConfigMap) -> Vec<String> {
        let entries = config_map.metadata.managed_fields.clone().unwrap();
        entries
            .iter()
            .map(|entry| serde_json::to_value(entry).unwrap()["time"].to_string())
            .collect()
    }

    /// Applying the same manifest again changes nothing, with one manager or with
    /// two, where the applier's entry moves past the other one.
    #[tokio::test]
    async fn test_repeated_apply_is_a_noop() {
        let only_a = config_map_manifest(json!({ "a": "1" }));
        let only_b = config_map_manifest(json!({ "b": "2" }));
        let seeds = [
            (json!({ "a": "1" }), vec![seeded_entry("a", &only_a)]),
            (
                json!({ "a": "1", "b": "2" }),
                vec![seeded_entry("a", &only_a), seeded_entry("b", &only_b)],
            ),
        ];

        for (data, entries) in seeds {
            let config_maps = seeded_config_maps(data, entries).await;
            let seeded = config_maps.get("applied").await.unwrap();

            let applied = config_maps
                .patch("applied", &PatchParams::apply("a"), &Patch::Apply(&only_a))
                .await
                .unwrap();

            assert_eq!(
                applied.metadata.resource_version,
                seeded.metadata.resource_version
            );
            assert_eq!(entry_times(&applied), entry_times(&seeded));
        }
    }

    /// An apply that changes a value is a real write and takes a new timestamp.
    #[tokio::test]
    async fn test_apply_that_changes_a_value_is_written() {
        let manifest = config_map_manifest(json!({ "a": "1" }));
        let config_maps =
            seeded_config_maps(json!({ "a": "1" }), vec![seeded_entry("a", &manifest)]).await;
        let seeded = config_maps.get("applied").await.unwrap();

        let applied = config_maps
            .patch(
                "applied",
                &PatchParams::apply("a"),
                &Patch::Apply(&config_map_manifest(json!({ "a": "2" }))),
            )
            .await
            .unwrap();

        assert_ne!(
            applied.metadata.resource_version,
            seeded.metadata.resource_version
        );
        assert_ne!(entry_times(&applied), entry_times(&seeded));
    }

    /// A client that doesn't know about `managedFields` can't remove them.
    #[tokio::test]
    async fn test_replace_without_managed_fields_keeps_them() {
        let manifest = config_map_manifest(json!({ "a": "1" }));
        let config_maps =
            seeded_config_maps(json!({ "a": "1" }), vec![seeded_entry("a", &manifest)]).await;
        let seeded = config_maps.get("applied").await.unwrap();

        let mut update = seeded.clone();
        update.metadata.managed_fields = None;
        update.data = Some([("a".to_string(), "2".to_string())].into());
        let replaced = config_maps
            .replace("applied", &PostParams::default(), &update)
            .await
            .unwrap();

        assert_ne!(
            replaced.metadata.resource_version,
            seeded.metadata.resource_version
        );
        assert_eq!(
            replaced.metadata.managed_fields,
            seeded.metadata.managed_fields
        );
    }

    /// A status write can't change `managedFields`, whatever the request sends.
    #[tokio::test]
    async fn test_status_writes_ignore_managed_fields() {
        let manifest = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": { "name": "web", "namespace": "default" },
        });
        let mut seeded = manifest.clone();
        seeded["metadata"]["labels"] = json!({ "app": "web" });
        seeded["metadata"]["managedFields"] = json!([seeded_entry("a", &seeded)]);
        let client = ClientBuilder::new()
            .with_runtime_objects(vec![seeded])
            .build()
            .await
            .unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client.clone(), "default");
        let expected = pods.get("web").await.unwrap().metadata.managed_fields;
        assert!(expected.is_some());

        let requests = [
            json!(null),
            json!([]),
            json!([{ "manager": "other", "operation": "Update" }]),
        ];
        for (phase, managed_fields) in ["Running", "Pending", "Failed"].iter().zip(requests) {
            let mut body = manifest.clone();
            body["metadata"]["managedFields"] = managed_fields.clone();
            body["status"] = json!({ "phase": phase });
            let request = http::Request::put("/api/v1/namespaces/default/pods/web/status")
                .header("content-type", "application/json")
                .body(serde_json::to_vec(&body).unwrap())
                .unwrap();
            let replaced: Pod = client.request(request).await.unwrap();
            assert_eq!(replaced.metadata.managed_fields, expected);

            let patch = json!({
                "metadata": { "managedFields": managed_fields },
                "status": { "phase": "Succeeded" },
            });
            let patched = pods
                .patch_status("web", &PatchParams::default(), &Patch::Merge(&patch))
                .await
                .unwrap();
            assert_eq!(patched.metadata.managed_fields, expected);
        }
    }

    /// An empty list or a single empty entry clears `managedFields`.
    #[tokio::test]
    async fn test_replace_can_reset_managed_fields() {
        let manifest = config_map_manifest(json!({ "a": "1" }));
        for reset in [vec![], vec![ManagedFieldsEntry::default()]] {
            let config_maps =
                seeded_config_maps(json!({ "a": "1" }), vec![seeded_entry("a", &manifest)]).await;
            let seeded = config_maps.get("applied").await.unwrap();

            let mut update = seeded.clone();
            update.metadata.managed_fields = Some(reset);
            let replaced = config_maps
                .replace("applied", &PostParams::default(), &update)
                .await
                .unwrap();

            assert_ne!(
                replaced.metadata.resource_version,
                seeded.metadata.resource_version
            );
            assert_eq!(replaced.metadata.managed_fields, None);
        }
    }

    /// The scale subresource round-trips through a Deployment.
    #[tokio::test]
    async fn test_deployment_scale_round_trip() {
        use k8s_openapi::api::apps::v1::Deployment;

        let client = ClientBuilder::new().build().await.unwrap();
        let deployments: kube::Api<Deployment> = kube::Api::namespaced(client, "default");
        let created = deployments
            .create(&PostParams::default(), &web_deployment())
            .await
            .unwrap();

        let scale = deployments.get_scale("web").await.unwrap();
        assert_eq!(scale.metadata.name.as_deref(), Some("web"));
        assert_eq!(scale.metadata.uid, created.metadata.uid);
        assert_eq!(
            scale.metadata.resource_version,
            created.metadata.resource_version
        );
        assert_eq!(scale.spec.as_ref().and_then(|s| s.replicas), Some(1));
        let status = scale.status.clone().unwrap();
        assert_eq!(status.replicas, 0);
        assert_eq!(status.selector.as_deref(), Some("app=web"));

        let mut desired = scale;
        desired.spec.as_mut().unwrap().replicas = Some(3);
        let replaced = deployments
            .replace_scale("web", &PostParams::default(), &desired)
            .await
            .unwrap();
        assert_eq!(replaced.spec.and_then(|s| s.replicas), Some(3));

        let patched = deployments
            .patch_scale(
                "web",
                &PatchParams::default(),
                &Patch::Merge(json!({ "spec": { "replicas": 5 } })),
            )
            .await
            .unwrap();
        assert_eq!(patched.spec.and_then(|s| s.replicas), Some(5));

        let parent = deployments.get("web").await.unwrap();
        let spec = spec_of::<DeploymentSpec>(parent.spec.clone());
        assert_eq!(spec.replicas, Some(5));
        assert_eq!(
            spec.template,
            spec_of::<DeploymentSpec>(created.spec.clone()).template
        );
        assert_eq!(parent.metadata.labels, created.metadata.labels);
        assert_eq!(parent.status, created.status);
    }

    /// StatefulSet and ReplicaSet serve the scale subresource too.
    #[tokio::test]
    async fn test_statefulset_and_replicaset_scale() {
        use k8s_openapi::api::apps::v1::{ReplicaSet, StatefulSet};

        let client = ClientBuilder::new().build().await.unwrap();
        let spec = json!({
            "replicas": 2,
            "selector": { "matchLabels": { "app": "db" } },
            "template": {
                "metadata": { "labels": { "app": "db" } },
                "spec": { "containers": [{ "name": "db", "image": "postgres" }] }
            }
        });

        let statefulsets: kube::Api<StatefulSet> = kube::Api::namespaced(client.clone(), "default");
        let sts: StatefulSet = serde_json::from_value(json!({
            "apiVersion": "apps/v1", "kind": "StatefulSet",
            "metadata": { "name": "db" }, "spec": spec
        }))
        .unwrap();
        statefulsets
            .create(&PostParams::default(), &sts)
            .await
            .unwrap();
        let scale = statefulsets.get_scale("db").await.unwrap();
        assert_eq!(scale.spec.and_then(|s| s.replicas), Some(2));
        assert_eq!(
            scale.status.and_then(|s| s.selector).as_deref(),
            Some("app=db")
        );
        statefulsets
            .patch_scale(
                "db",
                &PatchParams::default(),
                &Patch::Merge(json!({ "spec": { "replicas": 4 } })),
            )
            .await
            .unwrap();
        assert_eq!(
            spec_of::<StatefulSetSpec>(statefulsets.get("db").await.unwrap().spec).replicas,
            Some(4)
        );

        let replicasets: kube::Api<ReplicaSet> = kube::Api::namespaced(client, "default");
        let rs: ReplicaSet = serde_json::from_value(json!({
            "apiVersion": "apps/v1", "kind": "ReplicaSet",
            "metadata": { "name": "db" }, "spec": spec
        }))
        .unwrap();
        replicasets
            .create(&PostParams::default(), &rs)
            .await
            .unwrap();
        assert_eq!(
            replicasets
                .get_scale("db")
                .await
                .unwrap()
                .spec
                .and_then(|s| s.replicas),
            Some(2)
        );
        replicasets
            .patch_scale(
                "db",
                &PatchParams::default(),
                &Patch::Merge(json!({ "spec": { "replicas": 0 } })),
            )
            .await
            .unwrap();
        assert_eq!(
            spec_of::<ReplicaSetSpec>(replicasets.get("db").await.unwrap().spec).replicas,
            Some(0)
        );
    }

    /// Scale writes validate replicas, the uid precondition and the resource version.
    #[tokio::test]
    async fn test_scale_write_validation() {
        use k8s_openapi::api::apps::v1::Deployment;

        let client = ClientBuilder::new().build().await.unwrap();
        let deployments: kube::Api<Deployment> = kube::Api::namespaced(client, "default");
        deployments
            .create(&PostParams::default(), &web_deployment())
            .await
            .unwrap();
        let scale = deployments.get_scale("web").await.unwrap();

        let expect_code = |result: Result<_, kube::Error>, code: u16| match result {
            Err(kube::Error::Api(err)) => assert_eq!(err.code, code, "{err:?}"),
            other => panic!("expected {code}, got {other:?}"),
        };

        let mut negative = scale.clone();
        negative.spec.as_mut().unwrap().replicas = Some(-1);
        let result = deployments
            .replace_scale("web", &PostParams::default(), &negative)
            .await;
        if let Err(kube::Error::Api(err)) = &result {
            assert_eq!(
                err.message,
                "Scale.autoscaling \"web\" is invalid: spec.replicas: Invalid value: -1: must be greater than or equal to 0"
            );
        }
        expect_code(result, 422);

        let mut wrong_uid = scale.clone();
        wrong_uid.metadata.uid = Some("other".to_string());
        expect_code(
            deployments
                .replace_scale("web", &PostParams::default(), &wrong_uid)
                .await,
            409,
        );

        let mut stale = scale.clone();
        stale.metadata.resource_version = Some("1".to_string());
        deployments
            .patch_scale(
                "web",
                &PatchParams::default(),
                &Patch::Merge(json!({ "spec": { "replicas": 2 } })),
            )
            .await
            .unwrap();
        expect_code(
            deployments
                .replace_scale("web", &PostParams::default(), &stale)
                .await,
            409,
        );

        assert_eq!(
            spec_of::<DeploymentSpec>(deployments.get("web").await.unwrap().spec).replicas,
            Some(2)
        );
    }

    mod widget {
        use kube::CustomResource;
        use schemars::JsonSchema;
        use serde::{Deserialize, Serialize};

        #[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
        #[kube(
            group = "example.com",
            version = "v1",
            kind = "Widget",
            plural = "widgets",
            status = "WidgetStatus",
            namespaced
        )]
        pub struct WidgetSpec {
            pub replicas: i32,
        }

        #[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
        pub struct WidgetStatus {
            pub replicas: i32,
            pub selector: String,
        }

        pub fn new(name: &str) -> Widget {
            let mut widget = Widget::new(name, WidgetSpec { replicas: 2 });
            widget.status = Some(WidgetStatus {
                replicas: 1,
                selector: "app=widget".to_string(),
            });
            widget
        }
    }

    /// A custom resource serves `/scale` through the paths it registers.
    #[tokio::test]
    async fn test_custom_resource_scale_round_trip() {
        use widget::Widget;

        let client = ClientBuilder::new()
            .with_resource::<Widget>()
            .with_scale_subresource::<Widget>(
                ".spec.replicas",
                ".status.replicas",
                Some(".status.selector"),
            )
            .build()
            .await
            .unwrap();
        let widgets: kube::Api<Widget> = kube::Api::namespaced(client, "default");
        widgets
            .create(&PostParams::default(), &widget::new("w"))
            .await
            .unwrap();

        let scale = widgets.get_scale("w").await.unwrap();
        assert_eq!(scale.spec.and_then(|s| s.replicas), Some(2));
        let status = scale.status.unwrap();
        assert_eq!(status.replicas, 1);
        assert_eq!(status.selector.as_deref(), Some("app=widget"));

        widgets
            .patch_scale(
                "w",
                &PatchParams::default(),
                &Patch::Merge(json!({ "spec": { "replicas": 4 } })),
            )
            .await
            .unwrap();
        assert_eq!(widgets.get("w").await.unwrap().spec.replicas, 4);
    }

    /// A derived custom resource, which has no `Default`, can register a status subresource.
    #[tokio::test]
    async fn test_custom_resource_status_subresource_registration() {
        use widget::{Widget, WidgetStatus};

        let client = ClientBuilder::new()
            .with_resource::<Widget>()
            .with_status_subresource::<Widget>()
            .build()
            .await
            .unwrap();
        let widgets: kube::Api<Widget> = kube::Api::namespaced(client, "default");
        let mut current = widgets
            .create(&PostParams::default(), &widget::new("w"))
            .await
            .unwrap();

        current.status = Some(WidgetStatus {
            replicas: 9,
            selector: "changed".to_string(),
        });
        let replaced = widgets
            .replace("w", &PostParams::default(), &current)
            .await
            .unwrap();
        assert!(replaced.status.is_none());
    }

    /// A create drops the status of a kind with a status subresource, through
    /// POST and through a server-side apply that creates the object.
    #[tokio::test]
    async fn test_create_drops_submitted_status() {
        use k8s_openapi::api::apps::v1::{Deployment, DeploymentStatus};
        use widget::Widget;

        let client = ClientBuilder::new()
            .with_resource::<Widget>()
            .with_status_subresource::<Widget>()
            .build()
            .await
            .unwrap();
        let pp = PostParams::default();

        let deployments: kube::Api<Deployment> = kube::Api::namespaced(client.clone(), "default");
        let mut deployment = web_deployment();
        deployment.status = Some(DeploymentStatus {
            replicas: Some(7),
            ..Default::default()
        });
        let created = deployments.create(&pp, &deployment).await.unwrap();
        assert!(created.status.is_none());
        assert!(deployments.get("web").await.unwrap().status.is_none());

        let pods: kube::Api<Pod> = kube::Api::namespaced(client.clone(), "default");
        let applied = pods
            .patch(
                "applied",
                &PatchParams::apply("test"),
                &Patch::Apply(json!({
                    "apiVersion": "v1",
                    "kind": "Pod",
                    "metadata": { "name": "applied" },
                    "status": { "phase": "Running" }
                })),
            )
            .await
            .unwrap();
        assert!(applied.status.is_none());

        let widgets: kube::Api<Widget> = kube::Api::namespaced(client, "default");
        let created = widgets.create(&pp, &widget::new("w")).await.unwrap();
        assert!(created.status.is_none());
    }

    /// A Node keeps the status it is created with, and so does an object
    /// loaded with `with_object`.
    #[tokio::test]
    async fn test_status_kept_for_nodes_and_seeded_objects() {
        use k8s_openapi::api::apps::v1::{Deployment, DeploymentStatus};
        use k8s_openapi::api::core::v1::{NodeAddress, NodeStatus};

        let mut seeded = web_deployment();
        seeded.metadata.namespace = Some("default".to_string());
        seeded.status = Some(DeploymentStatus {
            replicas: Some(7),
            ..Default::default()
        });
        let client = ClientBuilder::new()
            .with_object(seeded)
            .build()
            .await
            .unwrap();
        let deployments: kube::Api<Deployment> = kube::Api::namespaced(client.clone(), "default");
        let stored = deployments.get("web").await.unwrap();
        assert_eq!(stored.status.and_then(|s| s.replicas), Some(7));

        let nodes: kube::Api<Node> = kube::Api::all(client);
        let status = NodeStatus {
            addresses: Some(vec![NodeAddress {
                address: "10.0.0.1".to_string(),
                type_: "InternalIP".to_string(),
            }]),
            ..Default::default()
        };
        let mut node = Node::default();
        node.metadata.name = Some("n1".to_string());
        node.status = Some(status.clone());
        let created = nodes.create(&PostParams::default(), &node).await.unwrap();
        assert_eq!(created.status, Some(status));
    }

    /// Without a selector path the `Scale` has no selector, and without
    /// registration a custom resource has no `/scale` path.
    #[tokio::test]
    async fn test_custom_resource_scale_registration() {
        use widget::Widget;

        let client = ClientBuilder::new()
            .with_resource::<Widget>()
            .with_scale_subresource::<Widget>(".spec.replicas", ".status.replicas", None)
            .build()
            .await
            .unwrap();
        let widgets: kube::Api<Widget> = kube::Api::namespaced(client, "default");
        widgets
            .create(&PostParams::default(), &widget::new("w"))
            .await
            .unwrap();
        let scale = widgets.get_scale("w").await.unwrap();
        assert_eq!(scale.status.unwrap().selector, None);

        let client = ClientBuilder::new()
            .with_resource::<Widget>()
            .build()
            .await
            .unwrap();
        let widgets: kube::Api<Widget> = kube::Api::namespaced(client, "default");
        widgets
            .create(&PostParams::default(), &widget::new("w"))
            .await
            .unwrap();
        assert_subresource_not_found(widgets.get_scale("w").await, "widgets/scale \"w\"");
    }

    /// A kind without a scale subresource has no `/scale` path.
    #[tokio::test]
    async fn test_scale_on_pod_returns_404() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");
        let mut pod = Pod::default();
        pod.metadata.name = Some("p".to_string());
        pods.create(&PostParams::default(), &pod).await.unwrap();

        assert_subresource_not_found(pods.get_scale("p").await, "pods/scale \"p\"");
    }

    /// Delete with `DeleteParams::orphan()` should NOT cascade to children.
    #[tokio::test]
    async fn test_delete_orphan_does_not_cascade() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let mut owner = Pod::default();
        owner.metadata.name = Some("owner-pod".to_string());
        let created = pods.create(&PostParams::default(), &owner).await.unwrap();
        let owner_uid = created.metadata.uid.unwrap();

        let mut child = Pod::default();
        child.metadata.name = Some("child-pod".to_string());
        child.metadata.owner_references = Some(vec![
            k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
                api_version: "v1".to_string(),
                kind: "Pod".to_string(),
                name: "owner-pod".to_string(),
                uid: owner_uid,
                ..Default::default()
            },
        ]);
        pods.create(&PostParams::default(), &child).await.unwrap();

        pods.delete("owner-pod", &DeleteParams::orphan())
            .await
            .unwrap();

        // Owner is gone
        assert!(pods.get("owner-pod").await.is_err());
        // Child should still exist because orphan does not cascade
        let child_result = pods.get("child-pod").await;
        assert!(child_result.is_ok(), "child should survive orphan delete");
    }

    fn pod_named(name: &str, finalizers: &[&str]) -> Pod {
        let mut pod = Pod::default();
        pod.metadata.name = Some(name.to_string());
        if !finalizers.is_empty() {
            pod.metadata.finalizers = Some(finalizers.iter().map(ToString::to_string).collect());
        }
        pod
    }

    fn owned_by(mut pod: Pod, owners: &[&Pod]) -> Pod {
        pod.metadata.owner_references = Some(
            owners
                .iter()
                .map(
                    |owner| k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
                        api_version: "v1".to_string(),
                        kind: "Pod".to_string(),
                        name: owner.metadata.name.clone().unwrap(),
                        uid: owner.metadata.uid.clone().unwrap(),
                        ..Default::default()
                    },
                )
                .collect(),
        );
        pod
    }

    async fn create_pod(pods: &kube::Api<Pod>, pod: Pod) -> Pod {
        pods.create(&PostParams::default(), &pod).await.unwrap()
    }

    async fn set_finalizers(pods: &kube::Api<Pod>, name: &str, finalizers: serde_json::Value) {
        pods.patch(
            name,
            &PatchParams::default(),
            &Patch::Merge(json!({ "metadata": { "finalizers": finalizers } })),
        )
        .await
        .unwrap();
    }

    /// Creates `owner` with `finalizers` and a `child` it owns.
    async fn owner_and_child(finalizers: &[&str]) -> (kube::Client, kube::Api<Pod>) {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client.clone(), "default");
        let owner = create_pod(&pods, pod_named("owner", finalizers)).await;
        create_pod(&pods, owned_by(pod_named("child", &[]), &[&owner])).await;
        (client, pods)
    }

    fn has_owner_references(pod: &Pod) -> bool {
        pod.metadata.owner_references.is_some()
    }

    /// Orphaning does not wait for finalizers, and removing them later leaves the child.
    #[tokio::test]
    async fn test_orphan_delete_with_finalizers_keeps_dependents() {
        let (_, pods) = owner_and_child(&["example.com/a", "example.com/b"]).await;

        pods.delete("owner", &DeleteParams::orphan()).await.unwrap();

        let owner = pods.get("owner").await.unwrap();
        assert!(owner.metadata.deletion_timestamp.is_some());
        assert_eq!(
            owner.metadata.finalizers,
            Some(vec![
                "example.com/a".to_string(),
                "example.com/b".to_string()
            ])
        );
        assert!(!has_owner_references(&pods.get("child").await.unwrap()));

        set_finalizers(&pods, "owner", json!(["example.com/b"])).await;
        assert!(pods.get("owner").await.is_ok());

        set_finalizers(&pods, "owner", json!(null)).await;
        assert!(pods.get("owner").await.is_err());
        assert!(pods.get("child").await.is_ok());
    }

    /// Orphaning removes only the deleted owner's entry.
    #[tokio::test]
    async fn test_orphan_delete_keeps_other_owners() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");
        let owner = create_pod(&pods, pod_named("owner", &[])).await;
        let other = create_pod(&pods, pod_named("other", &[])).await;
        create_pod(&pods, owned_by(pod_named("child", &[]), &[&owner, &other])).await;

        pods.delete("owner", &DeleteParams::orphan()).await.unwrap();

        let references = pods.get("child").await.unwrap().metadata.owner_references;
        let uids: Vec<_> = references.into_iter().flatten().map(|r| r.uid).collect();
        assert_eq!(uids, vec![other.metadata.uid.unwrap()]);
    }

    #[tokio::test]
    async fn test_orphan_delete_without_finalizers_removes_owner_reference() {
        let (_, pods) = owner_and_child(&[]).await;

        pods.delete("owner", &DeleteParams::orphan()).await.unwrap();

        assert!(pods.get("owner").await.is_err());
        assert!(!has_owner_references(&pods.get("child").await.unwrap()));
    }

    #[tokio::test]
    async fn test_orphan_dependents_option_orphans() {
        let (client, pods) = owner_and_child(&["example.com/a"]).await;

        let body = serde_json::to_vec(&json!({ "orphanDependents": true })).unwrap();
        let req = http::Request::delete("/api/v1/namespaces/default/pods/owner")
            .header("Content-Type", "application/json")
            .body(body)
            .unwrap();
        client.request_text(req).await.unwrap();

        assert!(!has_owner_references(&pods.get("child").await.unwrap()));
    }

    /// The response is the object before the garbage collector clears `orphan`.
    #[tokio::test]
    async fn test_orphan_delete_response_carries_orphan_finalizer() {
        for (finalizers, expected) in [
            (vec![], vec!["orphan"]),
            (vec!["example.com/a"], vec!["example.com/a", "orphan"]),
        ] {
            let (_, pods) = owner_and_child(&finalizers).await;

            let deleted = pods
                .delete("owner", &DeleteParams::orphan())
                .await
                .unwrap()
                .left()
                .unwrap();

            assert!(deleted.metadata.deletion_timestamp.is_some());
            assert_eq!(deleted.metadata.finalizers.unwrap(), expected);
        }
    }

    /// A stale copy of the response cannot be written back over the cleared `orphan`.
    #[tokio::test]
    async fn test_orphan_delete_response_is_older_than_stored_object() {
        let (_, pods) = owner_and_child(&["example.com/a"]).await;

        let deleted = pods
            .delete("owner", &DeleteParams::orphan())
            .await
            .unwrap()
            .left()
            .unwrap();

        let stored = pods.get("owner").await.unwrap();
        assert_ne!(
            deleted.metadata.resource_version,
            stored.metadata.resource_version
        );
    }

    #[tokio::test]
    async fn test_delete_of_object_carrying_orphan_finalizer_orphans() {
        let (_, pods) = owner_and_child(&["orphan"]).await;

        pods.delete("owner", &DeleteParams::default())
            .await
            .unwrap();

        assert!(pods.get("owner").await.is_err());
        assert!(!has_owner_references(&pods.get("child").await.unwrap()));
    }

    /// An explicit policy overrides the `orphan` finalizer and removes it.
    #[tokio::test]
    async fn test_background_delete_removes_orphan_finalizer() {
        let (_, pods) = owner_and_child(&["orphan", "example.com/a"]).await;

        pods.delete("owner", &DeleteParams::background())
            .await
            .unwrap();

        assert_eq!(
            pods.get("owner").await.unwrap().metadata.finalizers,
            Some(vec!["example.com/a".to_string()])
        );

        set_finalizers(&pods, "owner", json!(null)).await;
        assert!(pods.get("child").await.is_err());
    }

    #[tokio::test]
    async fn test_repeat_orphan_delete_orphans_terminating_object() {
        let (_, pods) = owner_and_child(&["example.com/a"]).await;

        pods.delete("owner", &DeleteParams::default())
            .await
            .unwrap();
        let terminating = pods.get("owner").await.unwrap();
        assert!(has_owner_references(&pods.get("child").await.unwrap()));

        pods.delete("owner", &DeleteParams::orphan()).await.unwrap();

        assert!(!has_owner_references(&pods.get("child").await.unwrap()));
        assert_eq!(
            pods.get("owner").await.unwrap().metadata.deletion_timestamp,
            terminating.metadata.deletion_timestamp
        );
    }

    #[tokio::test]
    async fn test_delete_with_finalizers_cascades_once_finalizers_are_removed() {
        for params in [DeleteParams::default(), DeleteParams::background()] {
            let (_, pods) = owner_and_child(&["example.com/a"]).await;

            pods.delete("owner", &params).await.unwrap();
            assert!(pods.get("child").await.is_ok());

            set_finalizers(&pods, "owner", json!(null)).await;
            assert!(pods.get("child").await.is_err());
        }
    }

    /// The garbage collector deletes a dependent carrying `orphan` with `Orphan`.
    #[tokio::test]
    async fn test_cascade_honors_orphan_finalizer_on_dependent() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");
        let owner = create_pod(&pods, pod_named("owner", &[])).await;
        let held_owner = create_pod(&pods, pod_named("held-owner", &[])).await;

        let middle = owned_by(pod_named("middle", &["orphan"]), &[&owner]);
        let middle = create_pod(&pods, middle).await;
        create_pod(&pods, owned_by(pod_named("child", &[]), &[&middle])).await;

        let held = owned_by(
            pod_named("held", &["orphan", "example.com/a"]),
            &[&held_owner],
        );
        let held = create_pod(&pods, held).await;
        create_pod(&pods, owned_by(pod_named("held-child", &[]), &[&held])).await;

        pods.delete("owner", &DeleteParams::default())
            .await
            .unwrap();
        pods.delete("held-owner", &DeleteParams::default())
            .await
            .unwrap();

        assert!(pods.get("middle").await.is_err());
        assert!(!has_owner_references(&pods.get("child").await.unwrap()));

        let held = pods.get("held").await.unwrap();
        assert!(held.metadata.deletion_timestamp.is_some());
        assert_eq!(
            held.metadata.finalizers,
            Some(vec!["example.com/a".to_string()])
        );
        assert!(!has_owner_references(
            &pods.get("held-child").await.unwrap()
        ));
    }

    /// The namespace controller deletes contents with an explicit `Background`.
    #[tokio::test]
    async fn test_namespace_delete_does_not_orphan_contents() {
        let client = ClientBuilder::new().build().await.unwrap();
        let namespaces: kube::Api<Namespace> = kube::Api::all(client.clone());
        let mut namespace = Namespace::default();
        namespace.metadata.name = Some("team-a".to_string());
        namespaces
            .create(&PostParams::default(), &namespace)
            .await
            .unwrap();

        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "team-a");
        let owner = create_pod(&pods, pod_named("owner", &["orphan"])).await;
        let child = owned_by(pod_named("child", &["example.com/a"]), &[&owner]);
        create_pod(&pods, child).await;

        namespaces
            .delete("team-a", &DeleteParams::default())
            .await
            .unwrap();

        assert!(pods.get("owner").await.is_err());
        assert!(has_owner_references(&pods.get("child").await.unwrap()));
    }

    /// Creates a `K` named `owner` and a Pod named `child` it owns.
    async fn resource_and_child<K>(namespace: &str) -> (kube::Api<K>, kube::Api<Pod>, K)
    where
        K: Resource<DynamicType = (), Scope = NamespaceResourceScope>
            + Clone
            + Debug
            + Default
            + DeserializeOwned
            + Serialize,
    {
        let client = ClientBuilder::new().build().await.unwrap();
        let owners: kube::Api<K> = kube::Api::namespaced(client.clone(), namespace);
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, namespace);

        let mut owner = K::default();
        owner.meta_mut().name = Some("owner".to_string());
        let owner = owners.create(&PostParams::default(), &owner).await.unwrap();

        let mut child = pod_named("child", &[]);
        child.metadata.owner_references = Some(vec![owner.owner_ref(&()).unwrap()]);
        create_pod(&pods, child).await;
        (owners, pods, owner)
    }

    async fn assert_delete_orphans_by_default<K>()
    where
        K: Resource<DynamicType = (), Scope = NamespaceResourceScope>
            + Clone
            + Debug
            + Default
            + DeserializeOwned
            + Serialize,
    {
        let (owners, pods, _) = resource_and_child::<K>("default").await;
        owners
            .delete("owner", &DeleteParams::default())
            .await
            .unwrap();
        assert!(owners.get("owner").await.is_err());
        assert!(!has_owner_references(&pods.get("child").await.unwrap()));

        let (owners, pods, _) = resource_and_child::<K>("default").await;
        owners
            .delete_collection(&DeleteParams::default(), &ListParams::default())
            .await
            .unwrap();
        assert!(owners.get("owner").await.is_err());
        assert!(!has_owner_references(&pods.get("child").await.unwrap()));
    }

    async fn assert_explicit_policy_cascades<K>()
    where
        K: Resource<DynamicType = (), Scope = NamespaceResourceScope>
            + Clone
            + Debug
            + Default
            + DeserializeOwned
            + Serialize,
    {
        for params in [DeleteParams::background(), DeleteParams::foreground()] {
            let (owners, pods, _) = resource_and_child::<K>("default").await;
            owners.delete("owner", &params).await.unwrap();
            assert!(pods.get("child").await.is_err());
        }
    }

    #[tokio::test]
    async fn test_job_and_replication_controller_orphan_by_default() {
        assert_delete_orphans_by_default::<Job>().await;
        assert_delete_orphans_by_default::<ReplicationController>().await;
    }

    #[tokio::test]
    async fn test_job_and_replication_controller_cascade_with_explicit_policy() {
        assert_explicit_policy_cascades::<Job>().await;
        assert_explicit_policy_cascades::<ReplicationController>().await;
    }

    #[tokio::test]
    async fn test_deployment_cascades_by_default() {
        let (owners, pods, _) = resource_and_child::<Deployment>("default").await;

        owners
            .delete("owner", &DeleteParams::default())
            .await
            .unwrap();

        assert!(pods.get("child").await.is_err());
    }

    /// The garbage collector deletes dependents with `Background`, not the kind's default.
    #[tokio::test]
    async fn test_cascade_reaching_job_removes_its_dependents() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client.clone(), "default");
        let jobs: kube::Api<Job> = kube::Api::namespaced(client, "default");

        let owner = create_pod(&pods, pod_named("owner", &[])).await;
        let mut job = Job::default();
        job.metadata.name = Some("job".to_string());
        job.metadata.owner_references = Some(vec![owner.owner_ref(&()).unwrap()]);
        let job = jobs.create(&PostParams::default(), &job).await.unwrap();
        let mut child = pod_named("child", &[]);
        child.metadata.owner_references = Some(vec![job.owner_ref(&()).unwrap()]);
        create_pod(&pods, child).await;

        let mut held = Job::default();
        held.metadata.name = Some("held".to_string());
        held.metadata.finalizers = Some(vec!["example.com/a".to_string()]);
        held.metadata.owner_references = Some(vec![owner.owner_ref(&()).unwrap()]);
        let held = jobs.create(&PostParams::default(), &held).await.unwrap();
        let mut held_child = pod_named("held-child", &[]);
        held_child.metadata.owner_references = Some(vec![held.owner_ref(&()).unwrap()]);
        create_pod(&pods, held_child).await;

        pods.delete("owner", &DeleteParams::default())
            .await
            .unwrap();

        assert!(jobs.get("job").await.is_err());
        assert!(pods.get("child").await.is_err());
        assert!(has_owner_references(&pods.get("held-child").await.unwrap()));
    }

    #[tokio::test]
    async fn test_namespace_delete_does_not_orphan_job_dependents() {
        let client = ClientBuilder::new().build().await.unwrap();
        let namespaces: kube::Api<Namespace> = kube::Api::all(client.clone());
        let mut namespace = Namespace::default();
        namespace.metadata.name = Some("team-a".to_string());
        namespaces
            .create(&PostParams::default(), &namespace)
            .await
            .unwrap();

        let pods: kube::Api<Pod> = kube::Api::namespaced(client.clone(), "team-a");
        let jobs: kube::Api<Job> = kube::Api::namespaced(client, "team-a");
        let mut job = Job::default();
        job.metadata.name = Some("job".to_string());
        let job = jobs.create(&PostParams::default(), &job).await.unwrap();
        let mut child = pod_named("child", &["example.com/a"]);
        child.metadata.owner_references = Some(vec![job.owner_ref(&()).unwrap()]);
        create_pod(&pods, child).await;

        namespaces
            .delete("team-a", &DeleteParams::default())
            .await
            .unwrap();

        assert!(jobs.get("job").await.is_err());
        assert!(has_owner_references(&pods.get("child").await.unwrap()));
    }

    /// The default applies again on each delete, so `orphan` is added and cleared again.
    #[tokio::test]
    async fn test_repeat_delete_of_held_job_changes_resource_version() {
        let client = ClientBuilder::new().build().await.unwrap();
        let jobs: kube::Api<Job> = kube::Api::namespaced(client, "default");
        let mut job = Job::default();
        job.metadata.name = Some("job".to_string());
        job.metadata.finalizers = Some(vec!["example.com/a".to_string()]);
        jobs.create(&PostParams::default(), &job).await.unwrap();

        jobs.delete("job", &DeleteParams::default()).await.unwrap();
        let first = jobs.get("job").await.unwrap();
        jobs.delete("job", &DeleteParams::default()).await.unwrap();
        let second = jobs.get("job").await.unwrap();

        assert_eq!(
            first.metadata.deletion_timestamp,
            second.metadata.deletion_timestamp
        );
        assert_ne!(
            first.metadata.resource_version,
            second.metadata.resource_version
        );
    }

    /// Delete with `DeleteParams::background()` should cascade to children.
    #[tokio::test]
    async fn test_delete_background_cascades() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let mut owner = Pod::default();
        owner.metadata.name = Some("owner-pod".to_string());
        let created = pods.create(&PostParams::default(), &owner).await.unwrap();
        let owner_uid = created.metadata.uid.unwrap();

        let mut child = Pod::default();
        child.metadata.name = Some("child-pod".to_string());
        child.metadata.owner_references = Some(vec![
            k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
                api_version: "v1".to_string(),
                kind: "Pod".to_string(),
                name: "owner-pod".to_string(),
                uid: owner_uid,
                ..Default::default()
            },
        ]);
        pods.create(&PostParams::default(), &child).await.unwrap();

        pods.delete("owner-pod", &DeleteParams::background())
            .await
            .unwrap();

        assert!(pods.get("owner-pod").await.is_err());
        assert!(
            pods.get("child-pod").await.is_err(),
            "child should be cascaded with background delete"
        );
    }

    /// Delete with `DeleteParams::foreground()` should cascade to children.
    #[tokio::test]
    async fn test_delete_foreground_cascades() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");

        let mut owner = Pod::default();
        owner.metadata.name = Some("owner-pod".to_string());
        let created = pods.create(&PostParams::default(), &owner).await.unwrap();
        let owner_uid = created.metadata.uid.unwrap();

        let mut child = Pod::default();
        child.metadata.name = Some("child-pod".to_string());
        child.metadata.owner_references = Some(vec![
            k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
                api_version: "v1".to_string(),
                kind: "Pod".to_string(),
                name: "owner-pod".to_string(),
                uid: owner_uid,
                ..Default::default()
            },
        ]);
        pods.create(&PostParams::default(), &child).await.unwrap();

        pods.delete("owner-pod", &DeleteParams::foreground())
            .await
            .unwrap();

        assert!(pods.get("owner-pod").await.is_err());
        assert!(
            pods.get("child-pod").await.is_err(),
            "child should be cascaded with foreground delete"
        );
    }

    /// `parse_delete_params` extracts propagationPolicy from query string.
    #[test]
    fn test_parse_delete_params_propagation_policy() {
        use crate::mock_service::MockService;

        let (policy, orphan) = MockService::parse_delete_params(Some("propagationPolicy=Orphan"));
        assert_eq!(policy.as_deref(), Some("Orphan"));
        assert_eq!(orphan, None);

        let (policy, orphan) =
            MockService::parse_delete_params(Some("propagationPolicy=Background"));
        assert_eq!(policy.as_deref(), Some("Background"));
        assert_eq!(orphan, None);
    }

    /// `parse_delete_params` extracts orphanDependents from query string.
    #[test]
    fn test_parse_delete_params_orphan_dependents() {
        use crate::mock_service::MockService;

        let (policy, orphan) = MockService::parse_delete_params(Some("orphanDependents=true"));
        assert_eq!(policy, None);
        assert_eq!(orphan, Some(true));

        let (policy, orphan) = MockService::parse_delete_params(Some("orphanDependents=false"));
        assert_eq!(policy, None);
        assert_eq!(orphan, Some(false));
    }

    /// `parse_delete_params` returns None for both when query is empty.
    #[test]
    fn test_parse_delete_params_empty() {
        use crate::mock_service::MockService;

        let (policy, orphan) = MockService::parse_delete_params(None);
        assert_eq!(policy, None);
        assert_eq!(orphan, None);
    }

    /// Both propagationPolicy and orphanDependents set in body returns 422.
    #[tokio::test]
    async fn test_delete_mutual_exclusivity_returns_422() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client.clone(), "default");

        let mut pod = Pod::default();
        pod.metadata.name = Some("excl-pod".to_string());
        pods.create(&PostParams::default(), &pod).await.unwrap();

        let body = serde_json::to_vec(&json!({
            "propagationPolicy": "Background",
            "orphanDependents": true
        }))
        .unwrap();

        let req = http::Request::delete("/api/v1/namespaces/default/pods/excl-pod")
            .header("Content-Type", "application/json")
            .body(body)
            .unwrap();

        let result = client.request_text(req).await;
        assert!(
            result.is_err(),
            "should return error for mutually exclusive options"
        );
        if let Err(kube::Error::Api(err)) = result {
            assert_eq!(err.code, 422);
        } else {
            panic!("expected kube::Error::Api with code 422");
        }
    }

    /// Invalid propagationPolicy value returns 422.
    #[tokio::test]
    async fn test_delete_invalid_propagation_policy_returns_422() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client.clone(), "default");

        let mut pod = Pod::default();
        pod.metadata.name = Some("invalid-policy-pod".to_string());
        pods.create(&PostParams::default(), &pod).await.unwrap();

        let body = serde_json::to_vec(&json!({
            "propagationPolicy": "InvalidPolicy"
        }))
        .unwrap();

        let req = http::Request::delete("/api/v1/namespaces/default/pods/invalid-policy-pod")
            .header("Content-Type", "application/json")
            .body(body)
            .unwrap();

        let result = client.request_text(req).await;
        assert!(result.is_err(), "should return error for invalid policy");
        if let Err(kube::Error::Api(err)) = result {
            assert_eq!(err.code, 422);
        } else {
            panic!("expected kube::Error::Api with code 422");
        }
    }

    /// Malformed JSON body on delete returns 400.
    #[tokio::test]
    async fn test_delete_malformed_body_returns_400() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client.clone(), "default");

        let mut pod = Pod::default();
        pod.metadata.name = Some("malformed-pod".to_string());
        pods.create(&PostParams::default(), &pod).await.unwrap();

        let req = http::Request::delete("/api/v1/namespaces/default/pods/malformed-pod")
            .header("Content-Type", "application/json")
            .body(b"{invalid json".to_vec())
            .unwrap();

        let result = client.request_text(req).await;
        assert!(result.is_err(), "should return error for malformed body");
        if let Err(kube::Error::Api(err)) = result {
            assert_eq!(err.code, 400);
        } else {
            panic!("expected kube::Error::Api with code 400");
        }
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

    // ============================================================================
    // Unimplemented Subresource Tests
    // ============================================================================

    /// Builds a client holding pod `p` in `default` with label `app=web`.
    async fn client_with_pod_p() -> (kube::Client, kube::Api<Pod>, Pod) {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client.clone(), "default");
        let mut pod = Pod::default();
        pod.metadata.name = Some("p".to_string());
        pod.metadata.labels = Some(BTreeMap::from([("app".to_string(), "web".to_string())]));
        let created = pods.create(&PostParams::default(), &pod).await.unwrap();
        (client, pods, created)
    }

    /// Asserts `result` is a 404 `NotFound` Status whose message contains `needle`.
    fn assert_subresource_not_found<T>(result: Result<T, kube::Error>, needle: &str) {
        if let Err(kube::Error::Api(err)) = result {
            assert_eq!(err.code, 404, "unexpected status: {err:?}");
            assert_eq!(err.reason, "NotFound");
            assert!(
                err.message.contains(needle),
                "message {:?} should contain {needle:?}",
                err.message
            );
        } else {
            panic!("expected kube::Error::Api with code 404");
        }
    }

    /// Asserts `result` is a 405 `MethodNotAllowed` Status.
    fn assert_subresource_method_not_allowed<T>(result: Result<T, kube::Error>) {
        if let Err(kube::Error::Api(err)) = result {
            assert_eq!(err.code, 405, "unexpected status: {err:?}");
            assert_eq!(err.reason, "MethodNotAllowed");
            assert_eq!(
                err.message,
                "the server does not allow this method on the requested resource"
            );
        } else {
            panic!("expected kube::Error::Api with code 405");
        }
    }

    fn labels_of(pod: &Pod) -> BTreeMap<String, String> {
        pod.metadata.labels.clone().unwrap_or_default()
    }

    /// Pod logs are not implemented and return 404.
    #[tokio::test]
    async fn test_logs_subresource_returns_404() {
        let (_client, pods, _created) = client_with_pod_p().await;
        let result = pods.logs("p", &LogParams::default()).await;
        assert_subresource_not_found(result, "pods/log \"p\"");
    }

    /// An unknown subresource on GET returns 404.
    #[tokio::test]
    async fn test_unknown_subresource_get_returns_404() {
        let (_client, pods, _created) = client_with_pod_p().await;
        let result = pods.get_subresource("nope", "p").await;
        assert_subresource_not_found(result, "pods/nope \"p\"");
    }

    /// Eviction returns 404 and leaves the parent pod untouched.
    #[tokio::test]
    async fn test_evict_subresource_returns_404_and_leaves_parent_untouched() {
        let (_client, pods, created) = client_with_pod_p().await;
        let result = pods.evict("p", &EvictParams::default()).await;
        assert_subresource_not_found(result, "pods/eviction \"p\"");
        assert_eq!(
            pods.get("p").await.unwrap().metadata.resource_version,
            created.metadata.resource_version
        );
    }

    /// PUT on an unimplemented subresource returns 404 and does not modify the parent.
    #[tokio::test]
    async fn test_replace_unimplemented_subresource_returns_404() {
        let (_client, pods, created) = client_with_pod_p().await;
        let mut modified = created.clone();
        modified.metadata.labels =
            Some(BTreeMap::from([("app".to_string(), "changed".to_string())]));
        let result = pods
            .replace_subresource(
                "ephemeralcontainers",
                "p",
                &PostParams::default(),
                &modified,
            )
            .await;
        assert_subresource_not_found(result, "pods/ephemeralcontainers \"p\"");
        assert_eq!(
            labels_of(&pods.get("p").await.unwrap()),
            BTreeMap::from([("app".to_string(), "web".to_string())])
        );
    }

    /// Apply patch on an unimplemented subresource returns 404 even without a fieldManager.
    #[tokio::test]
    async fn test_apply_patch_on_unimplemented_subresource_returns_404_not_422() {
        let (client, _pods, _created) = client_with_pod_p().await;
        let body = serde_json::to_vec(&json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": { "name": "p" }
        }))
        .unwrap();
        let req = http::Request::patch("/api/v1/namespaces/default/pods/p/resize")
            .header("Content-Type", "application/apply-patch+yaml")
            .body(body)
            .unwrap();
        let result = client.request_text(req).await;
        assert_subresource_not_found(result, "pods/resize \"p\"");
    }

    /// Merge patch on an unimplemented subresource returns 404 and does not modify the parent.
    #[tokio::test]
    async fn test_merge_patch_on_unimplemented_subresource_returns_404() {
        let (_client, pods, _created) = client_with_pod_p().await;
        let result = pods
            .patch_subresource(
                "resize",
                "p",
                &PatchParams::default(),
                &Patch::Merge(&json!({"metadata": {"labels": {"app": "changed"}}})),
            )
            .await;
        assert_subresource_not_found(result, "pods/resize \"p\"");
        assert_eq!(
            labels_of(&pods.get("p").await.unwrap()),
            BTreeMap::from([("app".to_string(), "web".to_string())])
        );
    }

    /// DELETE on the status subresource returns 405 and does not change the parent.
    #[tokio::test]
    async fn test_delete_on_status_subresource_returns_405() {
        let (client, pods, created) = client_with_pod_p().await;
        let req = http::Request::delete("/api/v1/namespaces/default/pods/p/status")
            .body(Vec::new())
            .unwrap();
        let result = client.request_text(req).await;
        assert_subresource_method_not_allowed(result);
        let after = pods.get("p").await.unwrap();
        assert_eq!(
            serde_json::to_value(after).unwrap(),
            serde_json::to_value(created).unwrap()
        );
    }

    /// POST on the status subresource returns 405 and leaves the parent unchanged.
    #[tokio::test]
    async fn test_post_on_status_subresource_returns_405() {
        let (client, pods, created) = client_with_pod_p().await;
        let req = http::Request::post("/api/v1/namespaces/default/pods/p/status")
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&created).unwrap())
            .unwrap();
        let result = client.request_text(req).await;
        assert_subresource_method_not_allowed(result);
        let after = pods.get("p").await.unwrap();
        assert_eq!(
            serde_json::to_value(after).unwrap(),
            serde_json::to_value(created).unwrap()
        );
    }

    /// POST on a different unimplemented subresource continues to return 404.
    #[tokio::test]
    async fn test_post_on_unimplemented_subresource_returns_404() {
        let (client, pods, created) = client_with_pod_p().await;
        let req = http::Request::post("/api/v1/namespaces/default/pods/p/resize")
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&created).unwrap())
            .unwrap();
        let result = client.request_text(req).await;
        assert_subresource_not_found(result, "pods/resize \"p\"");
        let after = pods.get("p").await.unwrap();
        assert_eq!(
            serde_json::to_value(after).unwrap(),
            serde_json::to_value(created).unwrap()
        );
    }

    /// Sends a bodyless `method` request to `path`.
    async fn request_without_body(
        client: &kube::Client,
        method: &str,
        path: &str,
    ) -> Result<String, kube::Error> {
        let req = http::Request::builder()
            .method(method)
            .uri(path)
            .body(Vec::new())
            .unwrap();
        client.request_text(req).await
    }

    /// POST and DELETE on `/scale` return 405 for a kind with a scale subresource
    /// and leave the parent unchanged, and 404 for a kind without one.
    #[tokio::test]
    async fn test_unsupported_methods_on_scale() {
        use k8s_openapi::api::apps::v1::Deployment;

        let (client, _pods, _created) = client_with_pod_p().await;
        let deployments: kube::Api<Deployment> = kube::Api::namespaced(client.clone(), "default");
        let created = deployments
            .create(&PostParams::default(), &web_deployment())
            .await
            .unwrap();

        for method in ["POST", "DELETE"] {
            let path = "/apis/apps/v1/namespaces/default/deployments/web/scale";
            assert_subresource_method_not_allowed(
                request_without_body(&client, method, path).await,
            );
            let path = "/api/v1/namespaces/default/pods/p/scale";
            assert_subresource_not_found(
                request_without_body(&client, method, path).await,
                "pods/scale \"p\"",
            );
        }
        assert_eq!(deployments.get("web").await.unwrap(), created);
    }

    /// POST and DELETE on the subresources of a custom resource return 405 when
    /// they are registered, leaving the parent unchanged, and 404 when they are not.
    #[tokio::test]
    async fn test_unsupported_methods_on_custom_resource_subresources() {
        use widget::Widget;

        let client = ClientBuilder::new()
            .with_resource::<Widget>()
            .with_status_subresource::<Widget>()
            .with_scale_subresource::<Widget>(".spec.replicas", ".status.replicas", None)
            .build()
            .await
            .unwrap();
        let widgets: kube::Api<Widget> = kube::Api::namespaced(client.clone(), "default");
        let created = widgets
            .create(&PostParams::default(), &widget::new("w"))
            .await
            .unwrap();
        let unregistered = ClientBuilder::new()
            .with_resource::<Widget>()
            .build()
            .await
            .unwrap();
        kube::Api::<Widget>::namespaced(unregistered.clone(), "default")
            .create(&PostParams::default(), &widget::new("w"))
            .await
            .unwrap();

        for sub in ["status", "scale"] {
            let path = format!("/apis/example.com/v1/namespaces/default/widgets/w/{sub}");
            for method in ["POST", "DELETE"] {
                assert_subresource_method_not_allowed(
                    request_without_body(&client, method, &path).await,
                );
                assert_subresource_not_found(
                    request_without_body(&unregistered, method, &path).await,
                    &format!("widgets/{sub} \"w\""),
                );
            }
        }
        assert_eq!(
            serde_json::to_value(widgets.get("w").await.unwrap()).unwrap(),
            serde_json::to_value(created).unwrap()
        );
    }

    /// A resource the fake does not know has no subresource paths, so every
    /// method on its `/status` or `/scale` returns 404.
    #[tokio::test]
    async fn test_subresources_of_unknown_resource_return_404() {
        let client = ClientBuilder::new().build().await.unwrap();

        for sub in ["status", "scale"] {
            let path = format!("/apis/example.com/v1/namespaces/default/widgets/w/{sub}");
            for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
                let req = http::Request::builder()
                    .method(method)
                    .uri(path.as_str())
                    .header("Content-Type", "application/merge-patch+json")
                    .body(b"{}".to_vec())
                    .unwrap();
                match client.request_text(req).await {
                    Err(kube::Error::Api(err)) => {
                        assert_eq!(err.code, 404, "{method} {path}: {err:?}")
                    }
                    other => panic!("{method} {path}: expected 404, got {other:?}"),
                }
            }
        }
    }

    /// Unimplemented subresources on cluster-scoped resources return 404.
    #[tokio::test]
    async fn test_cluster_scoped_unimplemented_subresource_returns_404() {
        let client = ClientBuilder::new().build().await.unwrap();
        let nodes: kube::Api<Node> = kube::Api::all(client);
        let mut node = Node::default();
        node.metadata.name = Some("n1".to_string());
        nodes.create(&PostParams::default(), &node).await.unwrap();
        let result = nodes.get_subresource("proxy", "n1").await;
        assert_subresource_not_found(result, "nodes/proxy \"n1\"");
    }

    /// Path segments after the subresource are part of the 404 message.
    #[tokio::test]
    async fn test_nested_subresource_path_returns_404() {
        let (client, _pods, _created) = client_with_pod_p().await;
        let req = http::Request::get("/api/v1/namespaces/default/pods/p/status/extra")
            .body(Vec::new())
            .unwrap();
        let result = client.request_text(req).await;
        assert_subresource_not_found(result, "pods/status/extra \"p\"");
    }

    /// A pod named `status` is routed as a normal object, not as a status subresource.
    #[tokio::test]
    async fn test_object_named_status_is_a_normal_object() {
        let client = ClientBuilder::new().build().await.unwrap();
        let pods: kube::Api<Pod> = kube::Api::namespaced(client, "default");
        let mut pod = Pod::default();
        pod.metadata.name = Some("status".to_string());
        pods.create(&PostParams::default(), &pod).await.unwrap();

        pods.patch(
            "status",
            &PatchParams::default(),
            &Patch::Merge(&json!({"metadata": {"labels": {"patched": "yes"}}})),
        )
        .await
        .unwrap();

        let fetched = pods.get("status").await.unwrap();
        assert_eq!(
            fetched
                .metadata
                .labels
                .as_ref()
                .unwrap()
                .get("patched")
                .map(String::as_str),
            Some("yes")
        );
    }
}
