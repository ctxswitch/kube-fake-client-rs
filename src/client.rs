//! Fake Kubernetes client for in-memory testing

use crate::client_utils::{extract_gvk, pluralize};
use crate::interceptor;
use crate::tracker::{ObjectTracker, GVK, GVR};
use crate::{Error, Result};
use kube::api::{ListParams, PatchParams, PostParams};
use kube::Resource;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
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
}

impl FakeClient {
    /// Create a new fake client with default settings
    pub fn new() -> Self {
        Self {
            tracker: Arc::new(ObjectTracker::new()),
            indexes: Arc::new(std::sync::RwLock::new(HashMap::new())),
            return_managed_fields: false,
            interceptors: None,
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

    /// Convert a Kubernetes resource to GVR from JSON value
    fn extract_gvr(&self, value: &Value) -> Result<GVR> {
        let gvk = extract_gvk(value)?;
        Ok(GVR::new(gvk.group, gvk.version, pluralize(&gvk.kind)))
    }

    /// Create an object
    pub fn create<K>(&self, namespace: &str, obj: &K, _params: &PostParams) -> Result<K>
    where
        K: Resource + Serialize + DeserializeOwned + Clone,
    {
        let value = serde_json::to_value(obj)?;
        let gvr = self.extract_gvr(&value)?;
        let gvk = extract_gvk(&value)?;

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

        let values = self.tracker.list(&gvr, namespace)?;

        let mut results: Vec<K> = values
            .into_iter()
            .map(|v| serde_json::from_value(v))
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Apply label selector
        if let Some(label_selector) = &params.label_selector {
            results.retain(|obj| {
                let meta = obj.meta();
                if let Some(labels) = &meta.labels {
                    return self.match_label_selector(labels, label_selector);
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

    /// Match label selector (supports key=value format)
    fn match_label_selector(&self, labels: &BTreeMap<String, String>, selector: &str) -> bool {
        for requirement in selector.split(',') {
            let requirement = requirement.trim();
            if let Some((key, value)) = requirement.split_once('=') {
                let key = key.trim_end_matches('=');
                let value = value.trim();
                if labels.get(key) != Some(&value.to_string()) {
                    return false;
                }
            }
        }
        true
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

                    if let Some(indexer) = self.get_index(gvk, field) {
                        let obj_value = serde_json::to_value(&obj)?;
                        let values = indexer(&obj_value);

                        if !values.iter().any(|v| v == expected_value) {
                            matches = false;
                            break;
                        }
                    } else {
                        return Err(Error::IndexNotFound {
                            kind: format!("{:?}", gvk),
                            field: field.to_string(),
                        });
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

        let mut existing = self.tracker.get(&gvr, namespace, name)?;
        json_patch::merge(&mut existing, patch);

        let updated = self
            .tracker
            .update(&gvr, &gvk, existing, namespace, false)?;

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
        }
    }
}
