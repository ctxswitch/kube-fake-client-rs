#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::error::Error;
    use crate::managed_fields::apply_field_ownership;

    fn base_object() -> Value {
        json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {
                "name": "test",
                "namespace": "default"
            },
            "data": {
                "key1": "value1"
            }
        })
    }

    #[test]
    fn adds_managed_fields_entry_on_first_apply() {
        let mut obj = base_object();
        let patch = json!({ "data": { "key1": "value1" } });

        apply_field_ownership(&mut obj, &patch, "manager-a", false, "v1", "")
            .expect("first apply should succeed");

        let entries = obj["metadata"]["managedFields"]
            .as_array()
            .expect("managedFields should be an array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["manager"], "manager-a");
    }

    #[test]
    fn same_manager_re_apply_updates_entry_without_conflict() {
        let mut obj = base_object();
        let patch1 = json!({ "data": { "key1": "v1" } });
        let patch2 = json!({ "data": { "key1": "v2", "key2": "v2" } });

        apply_field_ownership(&mut obj, &patch1, "manager-a", false, "v1", "")
            .expect("first apply");
        apply_field_ownership(&mut obj, &patch2, "manager-a", false, "v1", "")
            .expect("re-apply same manager");

        let entries = obj["metadata"]["managedFields"]
            .as_array()
            .expect("managedFields should be an array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["manager"], "manager-a");

        // The trie should include both key1 and key2.
        let fields = &entries[0]["fieldsV1"];
        assert!(fields["f:data"]["f:key1"].is_object());
        assert!(fields["f:data"]["f:key2"].is_object());
    }

    #[test]
    fn different_managers_non_overlapping_fields_no_conflict() {
        let mut obj = base_object();
        let patch_a = json!({ "data": { "key1": "v1" } });
        let patch_b = json!({ "data": { "key2": "v2" } });

        apply_field_ownership(&mut obj, &patch_a, "manager-a", false, "v1", "").expect("apply a");
        apply_field_ownership(&mut obj, &patch_b, "manager-b", false, "v1", "")
            .expect("apply b should not conflict");

        let entries = obj["metadata"]["managedFields"]
            .as_array()
            .expect("managedFields should be an array");
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn different_managers_conflicting_field_force_false_returns_conflict() {
        let mut obj = base_object();
        let patch_a = json!({ "data": { "key1": "v1" } });
        let patch_b = json!({ "data": { "key1": "v2" } });

        apply_field_ownership(&mut obj, &patch_a, "manager-a", false, "v1", "").expect("apply a");
        let result = apply_field_ownership(&mut obj, &patch_b, "manager-b", false, "v1", "");

        match result {
            Err(Error::Conflict(msg)) => {
                assert!(
                    msg.contains("manager-a"),
                    "conflict message should mention the other manager: {msg}"
                );
            }
            other => panic!("expected Conflict error, got {other:?}"),
        }
    }

    #[test]
    fn force_true_takes_ownership_and_prunes_other_manager() {
        let mut obj = base_object();
        let patch_a = json!({ "data": { "key1": "v1", "key2": "v2" } });
        let patch_b = json!({ "data": { "key1": "override" } });

        apply_field_ownership(&mut obj, &patch_a, "manager-a", false, "v1", "").expect("apply a");
        apply_field_ownership(&mut obj, &patch_b, "manager-b", true, "v1", "")
            .expect("force apply b");

        let entries = obj["metadata"]["managedFields"]
            .as_array()
            .expect("managedFields should be an array");
        assert_eq!(entries.len(), 2);

        // manager-a should still own key2 but not key1.
        let a_entry = entries
            .iter()
            .find(|e| e["manager"] == "manager-a")
            .expect("manager-a entry should exist");
        let a_fields = &a_entry["fieldsV1"];
        assert!(
            a_fields["f:data"]["f:key2"].is_object(),
            "manager-a should still own key2"
        );
        assert!(
            a_fields["f:data"].get("f:key1").is_none(),
            "manager-a should no longer own key1"
        );
    }

    #[test]
    fn force_true_removes_entry_when_all_fields_taken() {
        let mut obj = base_object();
        let patch_a = json!({ "data": { "key1": "v1" } });
        let patch_b = json!({ "data": { "key1": "override" } });

        apply_field_ownership(&mut obj, &patch_a, "manager-a", false, "v1", "").expect("apply a");
        apply_field_ownership(&mut obj, &patch_b, "manager-b", true, "v1", "")
            .expect("force apply b");

        let entries = obj["metadata"]["managedFields"]
            .as_array()
            .expect("managedFields should be an array");
        assert_eq!(entries.len(), 1, "manager-a should be fully removed");
        assert_eq!(entries[0]["manager"], "manager-b");
    }

    #[test]
    fn subresource_separation_no_conflict() {
        let mut obj = base_object();
        let patch_main = json!({ "data": { "key1": "v1" } });
        let patch_status = json!({ "data": { "key1": "v1" } });

        apply_field_ownership(&mut obj, &patch_main, "manager-a", false, "v1", "")
            .expect("apply main");
        apply_field_ownership(&mut obj, &patch_status, "manager-a", false, "v1", "status")
            .expect("apply status should not conflict with main by same manager");

        let entries = obj["metadata"]["managedFields"]
            .as_array()
            .expect("managedFields should be an array");
        assert_eq!(entries.len(), 2, "main and status entries should coexist");

        let main_entry = entries
            .iter()
            .find(|e| e.get("subresource").is_none())
            .expect("main entry (no subresource key)");
        let status_entry = entries
            .iter()
            .find(|e| e["subresource"] == "status")
            .expect("status entry");

        assert_eq!(main_entry["manager"], "manager-a");
        assert_eq!(status_entry["manager"], "manager-a");
    }

    #[test]
    fn entry_has_correct_shape() {
        let mut obj = base_object();
        let patch = json!({ "data": { "key1": "v1" } });

        apply_field_ownership(&mut obj, &patch, "test-mgr", false, "v1", "").expect("apply");

        let entry = &obj["metadata"]["managedFields"][0];
        assert_eq!(entry["manager"], "test-mgr");
        assert_eq!(entry["operation"], "Apply");
        assert_eq!(entry["apiVersion"], "v1");
        assert_eq!(entry["fieldsType"], "FieldsV1");
        assert!(
            entry.get("subresource").is_none(),
            "main resource entry should not have subresource key"
        );
        assert!(
            entry["time"].as_str().is_some_and(|t| !t.is_empty()),
            "time should be a non-empty string"
        );
        assert!(
            entry["fieldsV1"].is_object(),
            "fieldsV1 should be an object"
        );
    }
}
