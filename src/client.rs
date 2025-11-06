//! Fake Kubernetes client for in-memory testing

use crate::client_utils::extract_gvk;
use crate::discovery::Discovery;
use crate::field_selectors::extract_preregistered_field_value;
use crate::gen::immutable::is_field_immutable;
use crate::interceptor;
use crate::label_selector;
use crate::registry::ResourceRegistry;
use crate::tracker::{ObjectTracker, GVK, GVR};
use crate::validator::SchemaValidator;
use crate::{Error, Result};
use kube::api::{ListParams, PatchParams, PostParams};
use kube::Resource;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Index function that extracts values from an object for indexing
pub type IndexerFunc = Arc<dyn Fn(&Value) -> Vec<String> + Send + Sync>;

/// Fake Kubernetes client for testing
pub struct FakeClient {
    /// Object tracker for storage
    pub(crate) tracker: Arc<ObjectTracker>,
    /// Registered indexes for field selectors
    pub(crate) indexes: Arc<std::sync::RwLock<HashMap<GVK, HashMap<String, IndexerFunc>>>>,
    /// Whether to return managed fields
    pub(crate) return_managed_fields: bool,
    /// Interceptor functions for customizing behavior
    pub(crate) interceptors: Option<Arc<interceptor::Funcs>>,
    /// Custom resource registry for CRD discovery
    pub(crate) registry: Arc<ResourceRegistry>,
    /// Schema validator for object validation (optional, no validation if None)
    pub(crate) validator: Option<Arc<dyn SchemaValidator>>,
}

impl FakeClient {
    /// Create a new fake client with default settings
    pub fn new() -> Self {
        Self {
            tracker: Arc::new(ObjectTracker::new()),
            indexes: Arc::new(std::sync::RwLock::new(HashMap::new())),
            return_managed_fields: false,
            interceptors: None,
            registry: Arc::new(ResourceRegistry::new()),
            validator: None,
        }
    }

    /// Get the object tracker
    pub fn tracker(&self) -> &Arc<ObjectTracker> {
        &self.tracker
    }

    /// Get an index function for a GVK and field
    pub fn get_index(&self, gvk: &GVK, field: &str) -> Option<IndexerFunc> {
        let indexes = self.indexes.read().unwrap();
        indexes.get(gvk)?.get(field).cloned()
    }

    /// Convert a Kubernetes resource to GVR from JSON value using Discovery + Registry
    fn extract_gvr(&self, value: &Value) -> Result<GVR> {
        let gvk = extract_gvk(value)?;
        Discovery::gvk_to_gvr_with_registry(&gvk, &self.registry).ok_or_else(|| {
            Error::ResourceNotRegistered {
                group: gvk.group.clone(),
                version: gvk.version.clone(),
                resource: format!("{} (kind)", gvk.kind),
            }
        })
    }

    /// Validate that a verb is supported for the given GVK
    ///
    /// For built-in resources, checks Discovery data.
    /// For CRDs (registered in registry), allows all standard verbs by default.
    pub(crate) fn validate_verb(&self, gvk: &GVK, verb: &str) -> Result<()> {
        // Check if this is a built-in resource (in Discovery)
        if Discovery::get_plural(gvk).is_some() {
            // Built-in resource - check if verb is supported
            if !Discovery::supports_verb(gvk, verb) {
                return Err(Error::VerbNotSupported {
                    verb: verb.to_string(),
                    kind: gvk.kind.clone(),
                });
            }
        } else if self
            .registry
            .lookup_by_kind(&gvk.group, &gvk.version, &gvk.kind)
            .is_some()
        {
            // CRD registered in registry - allow all standard verbs
            // This matches real Kubernetes behavior where CRDs support standard verbs by default
            let standard_verbs = [
                "create",
                "get",
                "list",
                "update",
                "patch",
                "delete",
                "deletecollection",
                "watch",
            ];
            if !standard_verbs.contains(&verb) {
                return Err(Error::VerbNotSupported {
                    verb: verb.to_string(),
                    kind: gvk.kind.clone(),
                });
            }
        } else {
            // Resource not found in Discovery or Registry - will fail elsewhere
            // Skip verb validation for now
        }
        Ok(())
    }

    /// Validate that no immutable fields have changed between old and new objects
    ///
    /// This recursively checks all fields in the object, comparing old and new values.
    /// Immutable fields are looked up based on the type being checked:
    /// - Top-level fields are checked against the resource Kind
    /// - Fields under "metadata" are checked against ObjectMeta
    /// - Fields under "spec" are checked against {Kind}Spec
    #[doc(hidden)] // Internal API exposed for testing
    pub fn validate_immutable_fields(
        &self,
        gvk: &GVK,
        old: &Value,
        new: &Value,
    ) -> Result<()> {
        // Check top-level fields against the resource Kind
        self.check_immutable_object(
            &gvk.group,
            &gvk.version,
            &gvk.kind,
            old,
            new,
            "",
        )?;

        // Check metadata fields against ObjectMeta
        if let (Some(old_meta), Some(new_meta)) = (old.get("metadata"), new.get("metadata")) {
            self.check_immutable_object(
                "",
                "v1",
                "ObjectMeta",
                old_meta,
                new_meta,
                "metadata",
            )?;
        }

        // Check spec fields against {Kind}Spec
        if let (Some(old_spec), Some(new_spec)) = (old.get("spec"), new.get("spec")) {
            let spec_kind = format!("{}Spec", gvk.kind);
            self.check_immutable_object(
                &gvk.group,
                &gvk.version,
                &spec_kind,
                old_spec,
                new_spec,
                "spec",
            )?;
        }

        Ok(())
    }

    /// Check immutable fields within a specific object (non-recursive on nested objects)
    fn check_immutable_object(
        &self,
        group: &str,
        version: &str,
        kind: &str,
        old: &Value,
        new: &Value,
        path_prefix: &str,
    ) -> Result<()> {
        // Only compare if both are objects
        let (old_obj, new_obj) = match (old.as_object(), new.as_object()) {
            (Some(o), Some(n)) => (o, n),
            _ => return Ok(()),
        };

        // Server-managed fields that should not be validated as immutable
        // These are set/updated by the server (tracker) and not controlled by the user
        let server_managed_fields = [
            "resourceVersion",  // Used for optimistic locking, managed by tracker
            "generation",       // Incremented by server on spec changes
            "uid",              // Set by server on create
            "creationTimestamp", // Set by server on create
        ];

        // Check each field in the new object
        for (field_name, new_value) in new_obj {
            // Skip metadata and spec at top level (handled separately)
            if path_prefix.is_empty() && (field_name == "metadata" || field_name == "spec" || field_name == "status") {
                continue;
            }

            // Skip server-managed fields - these are handled by the tracker
            if path_prefix == "metadata" && server_managed_fields.contains(&field_name.as_str()) {
                continue;
            }

            // Check if this field is immutable
            if is_field_immutable(group, version, kind, field_name) {
                // Compare old and new values
                if let Some(old_value) = old_obj.get(field_name) {
                    if old_value != new_value {
                        let full_path = if path_prefix.is_empty() {
                            field_name.to_string()
                        } else {
                            format!("{}.{}", path_prefix, field_name)
                        };
                        return Err(Error::ImmutableField { field: full_path });
                    }
                }
            }
        }

        Ok(())
    }

    /// Create an object
    pub fn create<K>(&self, namespace: &str, obj: &K, _params: &PostParams) -> Result<K>
    where
        K: Resource + Serialize + DeserializeOwned + Clone,
    {
        let value = serde_json::to_value(obj)?;
        let gvr = self.extract_gvr(&value)?;
        let gvk = extract_gvk(&value)?;

        // Validate that create verb is supported
        self.validate_verb(&gvk, "create")?;

        // Validate schema if validator is configured
        if let Some(validator) = &self.validator {
            validator.validate(&gvk.group, &gvk.version, &gvk.kind, &value)?;
        }

        let created = self.tracker.create(&gvr, &gvk, value, namespace)?;

        let mut result: K = serde_json::from_value(created)?;

        if !self.return_managed_fields {
            result.meta_mut().managed_fields = None;
        }

        Ok(result)
    }

    /// Get an object
    pub fn get<K>(&self, namespace: &str, name: &str) -> Result<K>
    where
        K: Resource + Serialize + DeserializeOwned + Default,
    {
        let dummy = K::default();
        let dummy_value = serde_json::to_value(&dummy)?;
        let gvr = self.extract_gvr(&dummy_value)?;
        let gvk = extract_gvk(&dummy_value)?;

        // Validate that get verb is supported
        self.validate_verb(&gvk, "get")?;

        let value = self.tracker.get(&gvr, namespace, name)?;

        let mut result: K = serde_json::from_value(value)?;

        if !self.return_managed_fields {
            result.meta_mut().managed_fields = None;
        }

        Ok(result)
    }

    /// Update an object (replaces the entire object)
    pub fn update<K>(&self, namespace: &str, obj: &K, _params: &PostParams) -> Result<K>
    where
        K: Resource + Serialize + DeserializeOwned + Clone,
    {
        let value = serde_json::to_value(obj)?;
        let gvr = self.extract_gvr(&value)?;
        let gvk = extract_gvk(&value)?;

        // Validate that update verb is supported
        self.validate_verb(&gvk, "update")?;

        // Get the existing object to check for immutable field changes
        // In Kubernetes, the resource name comes from the URL path, not the request body.
        // Since we need to identify which resource to update, we use the name from the metadata.
        let name = obj.meta().name.as_ref().ok_or_else(|| {
            Error::InvalidRequest("resource name is required for update".to_string())
        })?;

        // Get the existing object by name, or search by UID if name lookup fails
        let existing = match self.tracker.get(&gvr, namespace, name) {
            Ok(obj) => obj,
            Err(Error::NotFound { .. }) => {
                // Name not found - check if there's an object with matching UID
                // This catches attempts to change the name (immutable field)
                if let Some(uid) = value.get("metadata")
                    .and_then(|m| m.get("uid"))
                    .and_then(|u| u.as_str())
                {
                    // Search all objects in this namespace for matching UID
                    let all_objects = self.tracker.list(&gvr, Some(namespace))?;
                    if all_objects.iter().any(|obj| {
                        obj.get("metadata")
                            .and_then(|m| m.get("uid"))
                            .and_then(|u| u.as_str()) == Some(uid)
                    }) {
                        // Found object with matching UID but different name
                        // This is an attempt to change the name (immutable)
                        return Err(Error::ImmutableField {
                            field: "metadata.name".to_string()
                        });
                    }
                }
                // No matching UID found, return the original NotFound error
                return Err(Error::NotFound {
                    kind: gvr.resource.clone(),
                    name: name.clone(),
                    namespace: namespace.to_string(),
                });
            }
            Err(e) => return Err(e),
        };

        // Validate that no immutable fields have changed
        self.validate_immutable_fields(&gvk, &existing, &value)?;

        // Validate schema if validator is configured
        if let Some(validator) = &self.validator {
            validator.validate(&gvk.group, &gvk.version, &gvk.kind, &value)?;
        }

        let updated = self.tracker.update(&gvr, &gvk, value, namespace, false)?;

        let mut result: K = serde_json::from_value(updated)?;

        if !self.return_managed_fields {
            result.meta_mut().managed_fields = None;
        }

        Ok(result)
    }

    /// Update the status subresource
    #[allow(dead_code)]
    pub fn update_status<K>(&self, namespace: &str, obj: &K, _params: &PostParams) -> Result<K>
    where
        K: Resource + Serialize + DeserializeOwned + Clone,
    {
        let value = serde_json::to_value(obj)?;
        let gvr = self.extract_gvr(&value)?;
        let gvk = extract_gvk(&value)?;

        // Validate that update verb is supported (status uses same verb)
        self.validate_verb(&gvk, "update")?;

        // Validate schema if validator is configured
        if let Some(validator) = &self.validator {
            validator.validate(&gvk.group, &gvk.version, &gvk.kind, &value)?;
        }

        let updated = self.tracker.update(&gvr, &gvk, value, namespace, true)?;

        let mut result: K = serde_json::from_value(updated)?;

        if !self.return_managed_fields {
            result.meta_mut().managed_fields = None;
        }

        Ok(result)
    }

    /// Delete an object
    #[allow(dead_code)]
    pub fn delete<K>(&self, namespace: &str, name: &str) -> Result<K>
    where
        K: Resource + Serialize + DeserializeOwned + Default,
    {
        let dummy = K::default();
        let dummy_value = serde_json::to_value(&dummy)?;
        let gvr = self.extract_gvr(&dummy_value)?;
        let gvk = extract_gvk(&dummy_value)?;

        // Validate that delete verb is supported
        self.validate_verb(&gvk, "delete")?;

        let value = self.tracker.delete(&gvr, namespace, name)?;

        let result: K = serde_json::from_value(value)?;
        Ok(result)
    }

    /// List objects
    pub fn list<K>(&self, namespace: Option<&str>, params: &ListParams) -> Result<Vec<K>>
    where
        K: Resource + Serialize + DeserializeOwned + Default,
    {
        let dummy = K::default();
        let dummy_value = serde_json::to_value(&dummy)?;
        let gvr = self.extract_gvr(&dummy_value)?;
        let gvk = extract_gvk(&dummy_value)?;

        // Validate that list verb is supported
        self.validate_verb(&gvk, "list")?;

        let values = self.tracker.list(&gvr, namespace)?;

        let mut results: Vec<K> = values
            .into_iter()
            .map(|v| serde_json::from_value(v))
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Apply label selector
        if let Some(label_selector_str) = &params.label_selector {
            results.retain(|obj| {
                let meta = obj.meta();
                if let Some(labels) = &meta.labels {
                    return label_selector::matches_label_selector(labels, label_selector_str)
                        .unwrap_or(false);
                }
                false
            });
        }

        // Apply field selector
        if let Some(field_selector) = &params.field_selector {
            results = self.filter_by_field_selector(results, &gvk, field_selector)?;
        }

        if !self.return_managed_fields {
            for obj in &mut results {
                obj.meta_mut().managed_fields = None;
            }
        }

        Ok(results)
    }

    /// Filter objects by field selector
    fn filter_by_field_selector<K>(
        &self,
        objects: Vec<K>,
        gvk: &GVK,
        selector: &str,
    ) -> Result<Vec<K>>
    where
        K: Resource + Serialize + DeserializeOwned,
    {
        let mut filtered = Vec::new();

        for obj in objects {
            let mut matches = true;

            for requirement in selector.split(',') {
                let requirement = requirement.trim();
                if let Some((field, expected_value)) = requirement.split_once('=') {
                    let field = field.trim_end_matches('=');
                    let expected_value = expected_value.trim();

                    let obj_value = serde_json::to_value(&obj)?;

                    // Try pre-registered fields first (no index required)
                    let values = if let Some(preregistered_values) =
                        extract_preregistered_field_value(&obj_value, field, &gvk.kind)
                    {
                        preregistered_values
                    } else if let Some(indexer) = self.get_index(gvk, field) {
                        // Fall back to custom registered index
                        indexer(&obj_value)
                    } else {
                        // Field not supported
                        return Err(Error::IndexNotFound {
                            kind: format!("{:?}", gvk),
                            field: field.to_string(),
                        });
                    };

                    if !values.iter().any(|v| v == expected_value) {
                        matches = false;
                        break;
                    }
                }
            }

            if matches {
                filtered.push(obj);
            }
        }

        Ok(filtered)
    }

    /// Patch an object
    pub fn patch<K>(
        &self,
        namespace: &str,
        name: &str,
        patch: &Value,
        _params: &PatchParams,
    ) -> Result<K>
    where
        K: Resource + Serialize + DeserializeOwned + Default,
    {
        let dummy = K::default();
        let dummy_value = serde_json::to_value(&dummy)?;
        let gvr = self.extract_gvr(&dummy_value)?;
        let gvk = extract_gvk(&dummy_value)?;

        // Validate that patch verb is supported
        self.validate_verb(&gvk, "patch")?;

        let existing = self.tracker.get(&gvr, namespace, name)?;
        let mut patched = existing.clone();
        json_patch::merge(&mut patched, patch);

        // Validate that no immutable fields have changed
        self.validate_immutable_fields(&gvk, &existing, &patched)?;

        // Validate the merged result
        if let Some(validator) = &self.validator {
            validator.validate(&gvk.group, &gvk.version, &gvk.kind, &patched)?;
        }

        let updated = self
            .tracker
            .update(&gvr, &gvk, patched, namespace, false)?;

        let mut result: K = serde_json::from_value(updated)?;

        if !self.return_managed_fields {
            result.meta_mut().managed_fields = None;
        }

        Ok(result)
    }
}

impl Default for FakeClient {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for FakeClient {
    fn clone(&self) -> Self {
        Self {
            tracker: Arc::clone(&self.tracker),
            indexes: Arc::clone(&self.indexes),
            return_managed_fields: self.return_managed_fields,
            interceptors: self.interceptors.clone(),
            registry: Arc::clone(&self.registry),
            validator: self.validator.clone(),
        }
    }
}
