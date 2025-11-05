use crate::utils::{
    deletion_timestamp_equal, ensure_metadata, increment_generation, should_be_deleted,
};
use crate::{Error, Result};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tracing::{debug, trace};

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GVR {
    pub group: String,
    pub version: String,
    pub resource: String,
}

impl GVR {
    pub fn new(
        group: impl Into<String>,
        version: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        Self {
            group: group.into(),
            version: version.into(),
            resource: resource.into(),
        }
    }

    pub fn not_found_error(&self, namespace: &str, name: &str) -> Error {
        Error::NotFound {
            kind: self.resource.clone(),
            name: name.to_string(),
            namespace: namespace.to_string(),
        }
    }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GVK {
    pub group: String,
    pub version: String,
    pub kind: String,
}

impl GVK {
    pub fn new(
        group: impl Into<String>,
        version: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        Self {
            group: group.into(),
            version: version.into(),
            kind: kind.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredObject {
    pub data: Value,
    pub gvk: GVK,
    pub metadata: ObjectMeta,
}

type ObjectsByName = HashMap<String, StoredObject>;
type ObjectsByNamespace = HashMap<String, ObjectsByName>;
type ObjectStorage = HashMap<GVR, ObjectsByNamespace>;

pub struct ObjectTracker {
    objects: Arc<RwLock<ObjectStorage>>,
    with_status_subresource: Arc<RwLock<std::collections::HashSet<GVK>>>,
    resource_version: Arc<AtomicU64>,
}

impl ObjectTracker {
    pub fn new() -> Self {
        Self {
            objects: Arc::new(RwLock::new(HashMap::new())),
            with_status_subresource: Arc::new(RwLock::new(std::collections::HashSet::new())),
            resource_version: Arc::new(AtomicU64::new(0)),
        }
    }

    fn next_resource_version(&self) -> String {
        let rv = self.resource_version.fetch_add(1, Ordering::SeqCst) + 1;
        rv.to_string()
    }

    pub fn add_status_subresource(&self, gvk: GVK) {
        let mut subresources = self.with_status_subresource.write().unwrap();
        subresources.insert(gvk);
    }

    pub fn has_status_subresource(&self, gvk: &GVK) -> bool {
        let subresources = self.with_status_subresource.read().unwrap();
        subresources.contains(gvk)
    }

    pub fn add(&self, gvr: &GVR, gvk: &GVK, mut object: Value, namespace: &str) -> Result<Value> {
        trace!("Adding object: {:?} in namespace: {}", gvr, namespace);

        let mut meta = self.extract_metadata(&object)?;

        let name = meta
            .name
            .clone()
            .ok_or_else(|| Error::InvalidRequest("Object name is required".to_string()))?;

        if meta.deletion_timestamp.is_some()
            && meta.finalizers.as_ref().is_none_or(|f| f.is_empty())
        {
            return Err(Error::InvalidRequest(format!(
                "refusing to add object {} with metadata.deletionTimestamp but no finalizers",
                name
            )));
        }

        if meta.resource_version.is_none()
            || meta
                .resource_version
                .as_ref()
                .is_none_or(|rv| rv.is_empty())
        {
            meta.resource_version = Some(self.next_resource_version());
        }

        ensure_metadata(&mut meta, namespace);

        object["metadata"] = serde_json::to_value(&meta)?;

        let stored = StoredObject {
            data: object.clone(),
            gvk: gvk.clone(),
            metadata: meta.clone(),
        };

        let mut objects = self.objects.write().unwrap();
        let gvr_objects = objects.entry(gvr.clone()).or_default();
        let ns_objects = gvr_objects.entry(namespace.to_string()).or_default();
        ns_objects.insert(name.clone(), stored);

        debug!("Added object: {}/{}", namespace, name);
        Ok(object)
    }

    pub fn create(
        &self,
        gvr: &GVR,
        gvk: &GVK,
        mut object: Value,
        namespace: &str,
    ) -> Result<Value> {
        trace!("Creating object: {:?} in namespace: {}", gvr, namespace);

        let mut meta = self.extract_metadata(&object)?;

        let name = meta
            .name
            .clone()
            .ok_or_else(|| Error::InvalidRequest("Object name is required".to_string()))?;

        if meta
            .resource_version
            .as_ref()
            .is_some_and(|rv| !rv.is_empty())
        {
            return Err(Error::InvalidRequest(
                "resourceVersion can not be set for Create requests".to_string(),
            ));
        }

        if self.get(gvr, namespace, &name).is_ok() {
            return Err(Error::AlreadyExists {
                kind: gvr.resource.clone(),
                name: name.clone(),
                namespace: namespace.to_string(),
            });
        }

        meta.resource_version = Some(self.next_resource_version());
        ensure_metadata(&mut meta, namespace);

        if meta.deletion_timestamp.is_some() {
            meta.deletion_timestamp = None;
        }

        object["metadata"] = serde_json::to_value(&meta)?;

        let stored = StoredObject {
            data: object.clone(),
            gvk: gvk.clone(),
            metadata: meta.clone(),
        };

        let mut objects = self.objects.write().unwrap();
        let gvr_objects = objects.entry(gvr.clone()).or_default();
        let ns_objects = gvr_objects.entry(namespace.to_string()).or_default();
        ns_objects.insert(name.clone(), stored);

        debug!("Created object: {}/{}", namespace, name);
        Ok(object)
    }

    pub fn get(&self, gvr: &GVR, namespace: &str, name: &str) -> Result<Value> {
        trace!("Getting object: {:?} {}/{}", gvr, namespace, name);

        let objects = self.objects.read().unwrap();
        let gvr_objects = objects
            .get(gvr)
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;

        let ns_objects = gvr_objects
            .get(namespace)
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;

        let stored = ns_objects
            .get(name)
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;

        Ok(stored.data.clone())
    }

    pub fn update(
        &self,
        gvr: &GVR,
        gvk: &GVK,
        mut object: Value,
        namespace: &str,
        is_status: bool,
    ) -> Result<Value> {
        trace!("Updating object: {:?} in namespace: {}", gvr, namespace);

        let meta = self.extract_metadata(&object)?;
        let name = meta
            .name
            .clone()
            .ok_or_else(|| Error::InvalidRequest("Object name is required".to_string()))?;

        let existing = self.get(gvr, namespace, &name)?;
        let existing_meta = self.extract_metadata(&existing)?;

        if let Some(provided_rv) = &meta.resource_version {
            if let Some(current_rv) = &existing_meta.resource_version {
                if provided_rv != current_rv && !provided_rv.is_empty() {
                    return Err(Error::Conflict(format!(
                        "Resource version mismatch: expected {}, got {}",
                        current_rv, provided_rv
                    )));
                }
            }
        }

        if self.has_status_subresource(gvk) {
            if is_status {
                if let Some(spec) = existing.get("spec") {
                    object["spec"] = spec.clone();
                }
            } else if let Some(status) = existing.get("status") {
                object["status"] = status.clone();
            }
        }

        let mut new_meta = self.extract_metadata(&object)?;
        new_meta.resource_version = Some(self.next_resource_version());
        new_meta.uid = existing_meta.uid;
        new_meta.creation_timestamp = existing_meta.creation_timestamp;

        // Increment generation when spec changes, but not for status-only updates
        if !is_status {
            new_meta.generation = Some(increment_generation(existing_meta.generation));
        } else {
            new_meta.generation = existing_meta.generation;
        }

        if !deletion_timestamp_equal(
            &new_meta.deletion_timestamp,
            &existing_meta.deletion_timestamp,
        ) {
            return Err(Error::InvalidRequest(
                "metadata.deletionTimestamp field is immutable".to_string(),
            ));
        }

        object["metadata"] = serde_json::to_value(&new_meta)?;

        if should_be_deleted(&new_meta) {
            return self.delete(gvr, namespace, &name);
        }

        let stored = StoredObject {
            data: object.clone(),
            gvk: gvk.clone(),
            metadata: new_meta.clone(),
        };

        let mut objects = self.objects.write().unwrap();
        let gvr_objects = objects
            .get_mut(gvr)
            .ok_or_else(|| gvr.not_found_error(namespace, &name))?;

        let ns_objects = gvr_objects
            .get_mut(namespace)
            .ok_or_else(|| gvr.not_found_error(namespace, &name))?;

        ns_objects.insert(name.clone(), stored);

        debug!("Updated object: {}/{}", namespace, name);
        Ok(object)
    }

    pub fn delete(&self, gvr: &GVR, namespace: &str, name: &str) -> Result<Value> {
        trace!("Deleting object: {:?} {}/{}", gvr, namespace, name);

        let mut objects = self.objects.write().unwrap();
        let gvr_objects = objects
            .get_mut(gvr)
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;

        let ns_objects = gvr_objects
            .get_mut(namespace)
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;

        let stored = ns_objects
            .remove(name)
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;

        debug!("Deleted object: {}/{}", namespace, name);
        Ok(stored.data)
    }

    pub fn list(&self, gvr: &GVR, namespace: Option<&str>) -> Result<Vec<Value>> {
        trace!("Listing objects: {:?} in namespace: {:?}", gvr, namespace);

        let objects = self.objects.read().unwrap();

        // If no objects of this type exist, return empty list instead of NotFound error
        // This matches Kubernetes API behavior
        let Some(gvr_objects) = objects.get(gvr) else {
            return Ok(Vec::new());
        };

        let result = match namespace {
            Some(ns) => gvr_objects
                .get(ns)
                .map(|objs| objs.values().map(|s| s.data.clone()).collect())
                .unwrap_or_default(),
            None => gvr_objects
                .values()
                .flat_map(|objs| objs.values().map(|s| s.data.clone()))
                .collect(),
        };

        Ok(result)
    }

    fn extract_metadata(&self, object: &Value) -> Result<ObjectMeta> {
        let meta_value = object
            .get("metadata")
            .ok_or_else(|| Error::MetadataError("Object missing metadata field".to_string()))?;

        serde_json::from_value(meta_value.clone())
            .map_err(|e| Error::MetadataError(format!("Failed to parse metadata: {}", e)))
    }
}

impl Default for ObjectTracker {
    fn default() -> Self {
        Self::new()
    }
}
