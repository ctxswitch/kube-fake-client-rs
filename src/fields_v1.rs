use serde_json::{Map, Value};

/// Top-level keys to skip when building the fieldsV1 trie.
const SKIP_TOP_LEVEL: &[&str] = &["apiVersion", "kind"];

/// Keys to skip when inside the `metadata` object.
const SKIP_METADATA: &[&str] = &[
    "managedFields",
    "resourceVersion",
    "uid",
    "creationTimestamp",
    "generation",
    "name",
    "namespace",
];

/// Build a `fieldsV1`-style trie from a JSON patch object.
///
/// Each key in the patch becomes an `f:key` entry. Object values are recursed
/// into; all other values (strings, numbers, booleans, arrays, null) produce
/// leaf nodes represented as empty objects `{}`.
///
/// Certain top-level keys (`apiVersion`, `kind`) and server-managed metadata
/// fields are skipped.
pub(crate) fn build_fields_v1(patch: &Value) -> Value {
    let Some(obj) = patch.as_object() else {
        return Value::Object(Map::new());
    };
    Value::Object(build_trie(obj, true, false))
}

fn build_trie(
    obj: &Map<String, Value>,
    is_top_level: bool,
    is_metadata: bool,
) -> Map<String, Value> {
    let mut result = Map::new();

    for (key, value) in obj {
        if is_top_level && SKIP_TOP_LEVEL.contains(&key.as_str()) {
            continue;
        }
        if is_metadata && SKIP_METADATA.contains(&key.as_str()) {
            continue;
        }

        let field_key = format!("f:{key}");
        match value.as_object() {
            Some(child) => {
                let child_is_metadata = is_top_level && key == "metadata";
                let subtree = build_trie(child, false, child_is_metadata);
                // Skip empty intermediate nodes (e.g. metadata where all
                // children were filtered out) to avoid claiming ownership
                // of the entire subtree as an atomic leaf.
                if !subtree.is_empty() {
                    result.insert(field_key, Value::Object(subtree));
                }
            }
            None => {
                result.insert(field_key, Value::Object(Map::new()));
            }
        }
    }

    result
}

/// Check whether two `fieldsV1` tries have any overlapping leaf paths.
///
/// A leaf in the trie is an empty object `{}`. Two tries overlap if they share
/// at least one complete path to a leaf.
pub(crate) fn fields_overlap(a: &Value, b: &Value) -> bool {
    let (Some(a_obj), Some(b_obj)) = (a.as_object(), b.as_object()) else {
        return false;
    };

    for (key, a_val) in a_obj {
        let Some(b_val) = b_obj.get(key) else {
            continue;
        };

        let a_is_leaf = a_val.as_object().is_some_and(Map::is_empty);
        let b_is_leaf = b_val.as_object().is_some_and(Map::is_empty);

        if a_is_leaf || b_is_leaf {
            // At least one side considers this a leaf -- overlap.
            return true;
        }

        // Both are non-empty objects -- recurse.
        if fields_overlap(a_val, b_val) {
            return true;
        }
    }

    false
}

/// Remove overlapping paths from `from` based on `remove`, returning a pruned trie.
///
/// For each key present in both tries:
/// - If `remove`'s value is a leaf `{}`, the key is dropped from `from`.
/// - If both values are non-empty objects, recurse. If the result is empty,
///   drop the key entirely.
///
/// Keys only in `from` are preserved unchanged.
pub(crate) fn subtract_fields(from: &Value, remove: &Value) -> Value {
    let (Some(from_obj), Some(remove_obj)) = (from.as_object(), remove.as_object()) else {
        return from.clone();
    };

    let mut result = Map::new();

    for (key, from_val) in from_obj {
        let Some(remove_val) = remove_obj.get(key) else {
            result.insert(key.clone(), from_val.clone());
            continue;
        };

        let remove_is_leaf = remove_val.as_object().is_some_and(Map::is_empty);
        if remove_is_leaf {
            // Remove this key entirely.
            continue;
        }

        let from_is_leaf = from_val.as_object().is_some_and(Map::is_empty);
        if from_is_leaf {
            // from owns this path as a leaf; remove has a deeper subtree.
            // Keep from's leaf -- the remove trie does not fully cover it.
            result.insert(key.clone(), from_val.clone());
            continue;
        }

        // Both present and non-leaf -- recurse if both are objects.
        if from_val.is_object() && remove_val.is_object() {
            let subtracted = subtract_fields(from_val, remove_val);
            if let Some(obj) = subtracted.as_object() {
                if !obj.is_empty() {
                    result.insert(key.clone(), subtracted);
                }
            }
        } else {
            // from_val is not an object but remove_val is non-leaf -- keep from_val.
            result.insert(key.clone(), from_val.clone());
        }
    }

    Value::Object(result)
}
