use serde_json::{Map, Value};

use crate::error::Error;
use crate::fields_v1::{build_fields_v1, fields_overlap, subtract_fields};

/// Apply field ownership tracking for server-side apply operations.
///
/// Updates `obj`'s `metadata.managedFields` to reflect ownership of fields
/// declared in `patch` by `field_manager`. Detects conflicts with other
/// managers and optionally forces ownership transfer when `force` is true.
pub(crate) fn apply_field_ownership(
    obj: &mut Value,
    patch: &Value,
    field_manager: &str,
    force: bool,
    api_version: &str,
    subresource: &str,
) -> Result<(), Error> {
    let patch_trie = build_fields_v1(patch);

    let existing_entries = read_managed_fields(obj);

    let mut updated_entries: Vec<Value> = Vec::with_capacity(existing_entries.len() + 1);

    for entry in &existing_entries {
        let entry_manager = entry.get("manager").and_then(Value::as_str).unwrap_or("");
        let entry_subresource = entry
            .get("subresource")
            .and_then(Value::as_str)
            .unwrap_or("");

        if entry_manager == field_manager && entry_subresource == subresource {
            // Our own entry — will be replaced by the new one below.
            continue;
        }

        // Entries for a different subresource manage an independent field set
        // and cannot conflict with this apply.
        if entry_subresource != subresource {
            updated_entries.push(entry.clone());
            continue;
        }

        let empty = Value::Object(Map::new());
        let entry_fields = entry.get("fieldsV1").unwrap_or(&empty);

        if fields_overlap(entry_fields, &patch_trie) {
            if !force {
                return Err(Error::Conflict(format!(
                    "Apply failed with 1 conflict: conflict with \"{entry_manager}\" using {api_version}"
                )));
            }

            let subtracted = subtract_fields(entry_fields, &patch_trie);
            let is_empty = subtracted.as_object().is_some_and(Map::is_empty);

            if is_empty {
                // Other manager loses all fields — drop entry entirely.
                continue;
            }

            let mut pruned_entry = entry.clone();
            pruned_entry
                .as_object_mut()
                .ok_or_else(|| Error::Internal("managedFields entry is not a JSON object".into()))?
                .insert("fieldsV1".to_string(), subtracted);
            updated_entries.push(pruned_entry);
        } else {
            updated_entries.push(entry.clone());
        }
    }

    let now = jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    let mut new_entry = serde_json::json!({
        "manager": field_manager,
        "operation": "Apply",
        "apiVersion": api_version,
        "fieldsType": "FieldsV1",
        "fieldsV1": patch_trie,
        "time": now,
    });
    if !subresource.is_empty() {
        new_entry["subresource"] = Value::String(subresource.to_string());
    }

    updated_entries.push(new_entry);

    ensure_metadata(obj)?;
    obj["metadata"]["managedFields"] = Value::Array(updated_entries);

    Ok(())
}

/// Read the existing `metadata.managedFields` array from an object.
/// Returns an empty vec if not present or not an array.
fn read_managed_fields(obj: &Value) -> Vec<Value> {
    obj.get("metadata")
        .and_then(|m| m.get("managedFields"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Ensure `obj["metadata"]` exists as an object.
fn ensure_metadata(obj: &mut Value) -> Result<(), Error> {
    if !obj.get("metadata").is_some_and(Value::is_object) {
        obj.as_object_mut()
            .ok_or_else(|| Error::Internal("resource is not a JSON object".into()))?
            .insert("metadata".to_string(), Value::Object(Map::new()));
    }
    Ok(())
}
