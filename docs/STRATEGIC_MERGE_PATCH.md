# Strategic Merge Patch with OpenAPI Schema Metadata

## Current State

The fake client currently treats Strategic Merge Patch like JSON Merge Patch (see `src/mock_service.rs:257-260`):

```rust
PatchType::StrategicMergePatch => {
    // For now, treat Strategic Merge Patch like Merge Patch
    // Full strategic merge would require schema knowledge
    json_patch::merge(existing, patch);
}
```

This means array merging doesn't behave like real Kubernetes - arrays are always replaced rather than merged intelligently.

## OpenAPI Schema Metadata Available

The Kubernetes OpenAPI schema (`kubernetes/api/openapi/swagger.json`) contains extensive patch-related metadata:

### Discovered Extensions
- **x-kubernetes-patch-strategy** (65 occurrences) - "merge", "replace", "merge,retainKeys"
- **x-kubernetes-patch-merge-key** (61 occurrences) - Which field to use as merge key
- **x-kubernetes-list-type** (381 occurrences) - "map", "atomic", "set"
- **x-kubernetes-list-map-keys** (73 occurrences) - Keys for list-map types
- **x-kubernetes-map-type** (43 occurrences) - "granular" or "atomic"

### Real Examples from Kubernetes Schema

#### Example 1: Pod Containers (List-Map Merge)
```json
{
  "containers": {
    "type": "array",
    "x-kubernetes-list-type": "map",
    "x-kubernetes-list-map-keys": ["name"],
    "x-kubernetes-patch-strategy": "merge",
    "x-kubernetes-patch-merge-key": "name",
    "items": {
      "$ref": "#/definitions/io.k8s.api.core.v1.Container"
    }
  }
}
```

**Meaning**: When patching containers, merge by the `name` field. If a container with the same name exists, update it; otherwise append it.

**Current Behavior** (wrong): Entire containers array is replaced
**Correct Behavior**: Containers merged by name

#### Example 2: Pod Volumes (Merge with RetainKeys)
```json
{
  "volumes": {
    "type": "array",
    "x-kubernetes-list-type": "map",
    "x-kubernetes-list-map-keys": ["name"],
    "x-kubernetes-patch-strategy": "merge,retainKeys",
    "x-kubernetes-patch-merge-key": "name"
  }
}
```

**Meaning**: Merge volumes by name, but with `retainKeys` strategy (more complex merging semantics).

#### Example 3: Pod Tolerations (Atomic List)
```json
{
  "tolerations": {
    "type": "array",
    "x-kubernetes-list-type": "atomic",
    "items": {
      "$ref": "#/definitions/io.k8s.api.core.v1.Toleration"
    }
  }
}
```

**Meaning**: Atomic list - always replace the entire array during patch.

**Current Behavior** (correct by accident): Array is replaced
**Correct Behavior**: Array is replaced

## How Strategic Merge Patch Should Work

### List Type Semantics

1. **x-kubernetes-list-type: "atomic"**
   - Replace the entire array
   - This is what JSON Merge Patch does by default

2. **x-kubernetes-list-type: "set"**
   - Treat as a set (no duplicates)
   - Merge by performing union
   - Order is preserved from original + new elements

3. **x-kubernetes-list-type: "map"**
   - Merge based on the key field(s)
   - If element with same key exists: update it
   - If element with new key: append it
   - Preserves order of existing elements

### Patch Strategy

1. **x-kubernetes-patch-strategy: "replace"**
   - Always replace the field entirely

2. **x-kubernetes-patch-strategy: "merge"**
   - For objects: deep merge
   - For arrays: use list-type semantics

3. **x-kubernetes-patch-strategy: "merge,retainKeys"**
   - Like merge, but only retain keys that appear in the patch
   - Removes keys not mentioned in patch (for specific fields)

## Real-World Impact

### Example: Patching Pod Containers

**Setup**: Pod with 2 containers
```yaml
spec:
  containers:
  - name: nginx
    image: nginx:1.14
  - name: sidecar
    image: sidecar:v1
```

**Patch**: Update nginx image
```yaml
spec:
  containers:
  - name: nginx
    image: nginx:1.21
```

**Current (Wrong) Behavior**:
```yaml
spec:
  containers:
  - name: nginx
    image: nginx:1.21
  # sidecar container LOST!
```

**Correct Behavior**:
```yaml
spec:
  containers:
  - name: nginx
    image: nginx:1.21  # Updated
  - name: sidecar
    image: sidecar:v1  # Preserved!
```

## Implementation Approach

To implement proper Strategic Merge Patch, we would need to:

### 1. Parse OpenAPI Schema for Merge Metadata

Create a utility to extract merge metadata from the OpenAPI schema:

```rust
struct MergeDirectives {
    list_type: Option<ListType>,
    list_map_keys: Vec<String>,
    patch_strategy: Option<PatchStrategy>,
    patch_merge_key: Option<String>,
}

enum ListType {
    Atomic,
    Set,
    Map,
}

enum PatchStrategy {
    Replace,
    Merge,
    MergeRetainKeys,
}
```

### 2. Implement Strategic Merge Algorithm

```rust
fn strategic_merge(
    existing: &mut Value,
    patch: &Value,
    schema_path: &[String],  // Path to current field in schema
    merge_directives: &HashMap<String, MergeDirectives>,
) -> Result<()> {
    match (existing, patch) {
        (Value::Array(existing_arr), Value::Array(patch_arr)) => {
            let directives = merge_directives.get(schema_path)?;

            match directives.list_type {
                Some(ListType::Atomic) => {
                    // Replace entire array
                    *existing_arr = patch_arr.clone();
                }
                Some(ListType::Map) => {
                    // Merge by key
                    merge_list_map(existing_arr, patch_arr, &directives.list_map_keys)?;
                }
                Some(ListType::Set) => {
                    // Merge as set
                    merge_list_set(existing_arr, patch_arr)?;
                }
                None => {
                    // Default to atomic
                    *existing_arr = patch_arr.clone();
                }
            }
        }
        (Value::Object(existing_obj), Value::Object(patch_obj)) => {
            // Deep merge objects
            for (key, patch_value) in patch_obj {
                let mut new_path = schema_path.to_vec();
                new_path.push(key.clone());

                if let Some(existing_value) = existing_obj.get_mut(key) {
                    strategic_merge(existing_value, patch_value, &new_path, merge_directives)?;
                } else {
                    existing_obj.insert(key.clone(), patch_value.clone());
                }
            }
        }
        _ => {
            // Scalar values: replace
            *existing = patch.clone();
        }
    }

    Ok(())
}

fn merge_list_map(
    existing: &mut Vec<Value>,
    patch: &[Value],
    keys: &[String],
) -> Result<()> {
    for patch_item in patch {
        // Extract key values from patch item
        let patch_key_values: Vec<&str> = keys
            .iter()
            .filter_map(|k| patch_item.get(k).and_then(|v| v.as_str()))
            .collect();

        // Find existing item with matching keys
        if let Some(existing_item) = existing.iter_mut().find(|item| {
            let item_key_values: Vec<&str> = keys
                .iter()
                .filter_map(|k| item.get(k).and_then(|v| v.as_str()))
                .collect();
            item_key_values == patch_key_values
        }) {
            // Merge with existing item
            json_patch::merge(existing_item, patch_item);
        } else {
            // Append new item
            existing.push(patch_item.clone());
        }
    }

    Ok(())
}
```

### 3. Integration Points

1. **Schema Loading**: Load and parse merge directives when `with_schema_validation_file()` is called
2. **Patch Application**: Use strategic merge when `PatchType::StrategicMergePatch` is detected
3. **Path Tracking**: Track the JSON path during merge to look up correct merge directives

## Benefits

1. **Test Accuracy**: Controllers tested with fake client will behave like production
2. **Bug Prevention**: Catches array merging bugs that would only appear in production
3. **Better Developer Experience**: Tests that work with fake client will work with real cluster
4. **CRD Support**: Custom resources can define their own merge strategies that will be respected

## Effort Estimate

- **Schema Parsing**: Load and extract merge metadata from OpenAPI definitions (~1-2 days)
- **List-Map Merge**: Implement merge by key for arrays (~1-2 days)
- **List-Set Merge**: Implement set union logic (~0.5-1 day)
- **Deep Object Merge**: Handle nested object merging with directives (~1 day)
- **Path Tracking**: Track JSON path during recursion (~0.5 day)
- **Testing**: Comprehensive tests for all merge strategies (~2-3 days)
- **Documentation**: Document the feature and examples (~0.5-1 day)

**Total**: ~6-10 days of development work

## Alternative: Partial Implementation

Start with just the most common case:

1. Implement only `x-kubernetes-list-type: "map"` with single merge keys
2. This covers the most critical use case (containers, volumes, etc.)
3. Fall back to current behavior for other cases
4. Add other strategies incrementally

This would provide 80% of the value with 20% of the effort (~2-3 days).
