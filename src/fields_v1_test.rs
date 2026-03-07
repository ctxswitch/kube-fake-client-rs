#[cfg(test)]
mod tests {
    use crate::fields_v1::*;
    use serde_json::json;

    #[test]
    fn build_fields_v1_produces_correct_trie_for_pod_like_patch() {
        let patch = json!({
            "metadata": {
                "name": "foo",
                "labels": {
                    "app": "bar"
                }
            },
            "spec": {
                "containers": [{"name": "nginx", "image": "nginx"}]
            }
        });

        let result = build_fields_v1(&patch);
        let expected = json!({
            "f:metadata": {
                "f:labels": {
                    "f:app": {}
                }
            },
            "f:spec": {
                "f:containers": {}
            }
        });

        assert_eq!(result, expected);
    }

    #[test]
    fn build_fields_v1_skips_api_version_kind_and_server_managed_metadata() {
        let patch = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "foo",
                "managedFields": [],
                "resourceVersion": "123",
                "uid": "abc-def",
                "creationTimestamp": "2024-01-01T00:00:00Z",
                "generation": 1
            }
        });

        let result = build_fields_v1(&patch);
        // All metadata keys are server-managed or identity fields — the empty
        // intermediate node is omitted to avoid claiming atomic ownership.
        let expected = json!({});

        assert_eq!(result, expected);
    }

    #[test]
    fn build_fields_v1_treats_arrays_as_atomic_leaves() {
        let patch = json!({
            "spec": {
                "volumes": [
                    {"name": "data", "emptyDir": {}}
                ],
                "replicas": 3
            }
        });

        let result = build_fields_v1(&patch);
        let expected = json!({
            "f:spec": {
                "f:volumes": {},
                "f:replicas": {}
            }
        });

        assert_eq!(result, expected);
    }

    #[test]
    fn build_fields_v1_returns_empty_object_for_non_object_input() {
        assert_eq!(build_fields_v1(&json!(42)), json!({}));
        assert_eq!(build_fields_v1(&json!("hello")), json!({}));
        assert_eq!(build_fields_v1(&json!(null)), json!({}));
        assert_eq!(build_fields_v1(&json!([1, 2, 3])), json!({}));
    }

    #[test]
    fn fields_overlap_returns_true_for_overlapping_leaf_paths() {
        let a = json!({
            "f:metadata": {
                "f:name": {}
            }
        });
        let b = json!({
            "f:metadata": {
                "f:name": {}
            }
        });

        assert!(fields_overlap(&a, &b));
    }

    #[test]
    fn fields_overlap_returns_false_for_non_overlapping_tries() {
        let a = json!({
            "f:metadata": {
                "f:name": {}
            }
        });
        let b = json!({
            "f:metadata": {
                "f:labels": {
                    "f:app": {}
                }
            }
        });

        assert!(!fields_overlap(&a, &b));
    }

    #[test]
    fn fields_overlap_handles_nested_overlap_correctly() {
        let a = json!({
            "f:spec": {
                "f:containers": {},
                "f:replicas": {}
            }
        });
        let b = json!({
            "f:spec": {
                "f:replicas": {}
            },
            "f:metadata": {
                "f:name": {}
            }
        });

        assert!(fields_overlap(&a, &b));
    }

    #[test]
    fn subtract_fields_removes_overlapping_paths() {
        let from = json!({
            "f:metadata": {
                "f:name": {},
                "f:labels": {
                    "f:app": {}
                }
            },
            "f:spec": {
                "f:replicas": {}
            }
        });
        let remove = json!({
            "f:spec": {
                "f:replicas": {}
            }
        });

        let result = subtract_fields(&from, &remove);
        let expected = json!({
            "f:metadata": {
                "f:name": {},
                "f:labels": {
                    "f:app": {}
                }
            }
        });

        assert_eq!(result, expected);
    }

    #[test]
    fn subtract_fields_handles_nested_subtraction() {
        let from = json!({
            "f:metadata": {
                "f:name": {},
                "f:labels": {
                    "f:app": {},
                    "f:env": {}
                }
            }
        });
        let remove = json!({
            "f:metadata": {
                "f:labels": {
                    "f:app": {}
                }
            }
        });

        let result = subtract_fields(&from, &remove);
        let expected = json!({
            "f:metadata": {
                "f:name": {},
                "f:labels": {
                    "f:env": {}
                }
            }
        });

        assert_eq!(result, expected);
    }

    #[test]
    fn subtract_fields_preserves_leaf_when_remove_has_deeper_subtree() {
        let from = json!({
            "f:metadata": {}
        });
        let remove = json!({
            "f:metadata": {
                "f:name": {}
            }
        });

        let result = subtract_fields(&from, &remove);
        let expected = json!({
            "f:metadata": {}
        });
        assert_eq!(result, expected);
    }

    #[test]
    fn subtract_fields_returns_empty_when_all_fields_removed() {
        let from = json!({
            "f:metadata": {
                "f:name": {}
            },
            "f:spec": {
                "f:replicas": {}
            }
        });
        let remove = json!({
            "f:metadata": {
                "f:name": {}
            },
            "f:spec": {
                "f:replicas": {}
            }
        });

        let result = subtract_fields(&from, &remove);
        assert_eq!(result, json!({}));
    }
}
