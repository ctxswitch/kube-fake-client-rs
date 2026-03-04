use crate::discovery::Discovery;
use crate::registry::ResourceRegistry;
use crate::utils::{
    deletion_timestamp_equal, ensure_metadata, increment_generation, should_be_deleted,
};
use crate::{Error, Result};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
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
    registry: Arc<ResourceRegistry>,
}

impl ObjectTracker {
    pub fn new(registry: Arc<ResourceRegistry>) -> Self {
        Self {
            objects: Arc::new(RwLock::new(HashMap::new())),
            with_status_subresource: Arc::new(RwLock::new(std::collections::HashSet::new())),
            resource_version: Arc::new(AtomicU64::new(0)),
            registry,
        }
    }

    fn next_resource_version(&self) -> String {
        let rv = self.resource_version.fetch_add(1, Ordering::SeqCst) + 1;
        rv.to_string()
    }

    pub fn add_status_subresource(&self, gvk: GVK) {
        self.with_status_subresource
            .write()
            .expect("lock poisoned")
            .insert(gvk);
    }

    pub fn has_status_subresource(&self, gvk: &GVK) -> bool {
        self.with_status_subresource
            .read()
            .expect("lock poisoned")
            .contains(gvk)
    }

    /// Validate that namespace usage matches the resource's scope.
    ///
    /// Returns an error if a namespaced resource is used without a namespace,
    /// or if a cluster-scoped resource is used with a namespace. Unknown
    /// resources (not in Discovery or the registry) pass validation.
    fn validate_namespace_scope(&self, gvk: &GVK, namespace: &str) -> Result<()> {
        let is_namespaced = Discovery::is_namespaced(gvk).or_else(|| {
            self.registry
                .is_namespaced(&gvk.group, &gvk.version, &gvk.kind)
        });

        match is_namespaced {
            Some(true) if namespace.is_empty() => Err(Error::InvalidRequest(format!(
                "namespaced resource {}/{} {} requires a namespace",
                gvk.group, gvk.version, gvk.kind
            ))),
            Some(false) if !namespace.is_empty() => Err(Error::InvalidRequest(format!(
                "cluster-scoped resource {}/{} {} cannot have a namespace",
                gvk.group, gvk.version, gvk.kind
            ))),
            _ => Ok(()),
        }
    }

    /// Auto-register status subresource if object has a status field
    fn maybe_register_status_subresource(&self, gvk: &GVK, object: &Value) {
        if object.get("status").is_some() {
            self.add_status_subresource(gvk.clone());
            debug!("Auto-registered status subresource for GVK: {:?}", gvk);
        }
    }

    /// Store object in tracker storage
    fn store_object(
        &self,
        gvr: &GVR,
        namespace: &str,
        name: &str,
        stored: StoredObject,
    ) -> Result<()> {
        let mut objects = self.objects.write().expect("lock poisoned");
        objects
            .entry(gvr.clone())
            .or_default()
            .entry(namespace.to_string())
            .or_default()
            .insert(name.to_string(), stored);
        Ok(())
    }

    /// Extract object name from metadata
    fn extract_name(meta: &ObjectMeta) -> Result<String> {
        meta.name
            .clone()
            .ok_or_else(|| Error::InvalidRequest("Object name is required".to_string()))
    }

    pub fn add(&self, gvr: &GVR, gvk: &GVK, mut object: Value, namespace: &str) -> Result<Value> {
        trace!("Adding object: {:?} in namespace: {}", gvr, namespace);
        self.validate_namespace_scope(gvk, namespace)?;

        let mut meta = self.extract_metadata(&object)?;
        let name = Self::extract_name(&meta)?;

        // Validate deletion timestamp without finalizers
        if meta.deletion_timestamp.is_some()
            && meta.finalizers.as_ref().is_none_or(|f| f.is_empty())
        {
            return Err(Error::InvalidRequest(format!(
                "refusing to add object {name} with metadata.deletionTimestamp but no finalizers"
            )));
        }

        // Set resource version if not present or empty
        if meta
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
            metadata: meta,
        };

        self.store_object(gvr, namespace, &name, stored)?;
        debug!("Added object: {}/{}", namespace, name);

        self.maybe_register_status_subresource(gvk, &object);

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
        self.validate_namespace_scope(gvk, namespace)?;

        let mut meta = self.extract_metadata(&object)?;
        let name = Self::extract_name(&meta)?;

        // Validate resource version not set for create
        if meta
            .resource_version
            .as_ref()
            .is_some_and(|rv| !rv.is_empty())
        {
            return Err(Error::InvalidRequest(
                "resourceVersion can not be set for Create requests".to_string(),
            ));
        }

        // Check if object already exists
        if self.get(gvr, namespace, &name).is_ok() {
            return Err(Error::AlreadyExists {
                kind: gvr.resource.clone(),
                name: name.clone(),
                namespace: namespace.to_string(),
            });
        }

        meta.resource_version = Some(self.next_resource_version());
        ensure_metadata(&mut meta, namespace);

        // Clear deletion timestamp if present
        if meta.deletion_timestamp.is_some() {
            meta.deletion_timestamp = None;
        }

        object["metadata"] = serde_json::to_value(&meta)?;

        let stored = StoredObject {
            data: object.clone(),
            gvk: gvk.clone(),
            metadata: meta,
        };

        self.store_object(gvr, namespace, &name, stored)?;
        debug!("Created object: {}/{}", namespace, name);

        self.maybe_register_status_subresource(gvk, &object);

        Ok(object)
    }

    pub fn get(&self, gvr: &GVR, namespace: &str, name: &str) -> Result<Value> {
        trace!("Getting object: {:?} {}/{}", gvr, namespace, name);

        let objects = self.objects.read().expect("lock poisoned");

        objects
            .get(gvr)
            .and_then(|gvr_objects| gvr_objects.get(namespace))
            .and_then(|ns_objects| ns_objects.get(name))
            .map(|stored| stored.data.clone())
            .ok_or_else(|| gvr.not_found_error(namespace, name))
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
        let name = Self::extract_name(&meta)?;

        let existing = self.get(gvr, namespace, &name)?;
        let existing_meta = self.extract_metadata(&existing)?;

        // Validate resource version for optimistic locking
        if let Some(provided_rv) = &meta.resource_version {
            if let Some(current_rv) = &existing_meta.resource_version {
                if provided_rv != current_rv && !provided_rv.is_empty() {
                    return Err(Error::Conflict(format!(
                        "Resource version mismatch: expected {current_rv}, got {provided_rv}"
                    )));
                }
            }
        }

        // Handle status subresource logic
        if self.has_status_subresource(gvk) {
            if is_status {
                // Status update: preserve spec
                if let Some(spec) = existing.get("spec") {
                    object["spec"] = spec.clone();
                }
            } else {
                // Regular update: preserve status
                if let Some(status) = existing.get("status") {
                    object["status"] = status.clone();
                }
            }
        }

        let mut new_meta = self.extract_metadata(&object)?;
        new_meta.resource_version = Some(self.next_resource_version());
        new_meta.uid = existing_meta.uid;
        new_meta.creation_timestamp = existing_meta.creation_timestamp;

        // Increment generation for spec changes, not for status-only updates
        new_meta.generation = if is_status {
            existing_meta.generation
        } else {
            Some(increment_generation(existing_meta.generation))
        };

        // Validate deletion timestamp immutability
        if !deletion_timestamp_equal(
            &new_meta.deletion_timestamp,
            &existing_meta.deletion_timestamp,
        ) {
            return Err(Error::InvalidRequest(
                "metadata.deletionTimestamp field is immutable".to_string(),
            ));
        }

        object["metadata"] = serde_json::to_value(&new_meta)?;

        // Delete if conditions are met
        if should_be_deleted(&new_meta) {
            return self.delete(gvr, namespace, &name, true);
        }

        let stored = StoredObject {
            data: object.clone(),
            gvk: gvk.clone(),
            metadata: new_meta,
        };

        let mut objects = self.objects.write().expect("lock poisoned");
        objects
            .get_mut(gvr)
            .and_then(|gvr_objects| gvr_objects.get_mut(namespace))
            .and_then(|ns_objects| ns_objects.insert(name.clone(), stored))
            .ok_or_else(|| gvr.not_found_error(namespace, &name))?;

        debug!("Updated object: {}/{}", namespace, name);
        Ok(object)
    }

    /// Deletes `name` and optionally cascades to all objects that transitively
    /// reference it via `ownerReferences`.
    ///
    /// When `cascade` is `true`, uses an iterative worklist under a single write
    /// lock: removes the target object, collects its UID, scans for dependents
    /// respecting Kubernetes scope rules (cluster-scoped owners scan all
    /// namespaces, namespaced owners scan only their own namespace), removes
    /// them, collects their UIDs, and repeats until no more dependents are found.
    /// A visited set prevents cycles.
    ///
    /// When `cascade` is `false`, only the target object is removed — no
    /// dependent scanning or deletion is performed.
    pub fn delete(&self, gvr: &GVR, namespace: &str, name: &str, cascade: bool) -> Result<Value> {
        trace!("Deleting object: {:?} {}/{}", gvr, namespace, name);

        let mut objects = self.objects.write().expect("lock poisoned");

        let stored = objects
            .get_mut(gvr)
            .and_then(|gvr_objects| gvr_objects.get_mut(namespace))
            .and_then(|ns_objects| ns_objects.remove(name))
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;

        debug!("Deleted object: {}/{}", namespace, name);
        let data = stored.data;

        if !cascade {
            return Ok(data);
        }

        let mut visited = HashSet::new();
        let mut pending = Vec::new();

        match stored.metadata.uid {
            Some(uid) => {
                visited.insert(uid.clone());
                pending.push((uid, namespace.to_string()));
            }
            None => {
                debug!(
                    "Cascade requested for {}/{} but object has no UID; dependents will not be removed",
                    namespace, name
                );
            }
        }

        while let Some((uid, owner_ns)) = pending.pop() {
            let dependents = Self::find_dependents_locked(&objects, &uid, &owner_ns);
            for (dep_gvr, dep_ns, dep_name) in dependents {
                if let Some(dep_stored) = objects
                    .get_mut(&dep_gvr)
                    .and_then(|g| g.get_mut(&dep_ns))
                    .and_then(|n| n.remove(&dep_name))
                {
                    debug!("Cascade-deleted dependent: {}/{}", dep_ns, dep_name);
                    if let Some(dep_uid) = dep_stored.metadata.uid {
                        if visited.insert(dep_uid.clone()) {
                            pending.push((dep_uid, dep_ns));
                        }
                    } else {
                        debug!(
                            "Cascade-deleted {}/{} has no UID; skipping transitive dependent scan",
                            dep_ns, dep_name
                        );
                    }
                }
            }
        }

        Ok(data)
    }

    /// Returns all objects in `objects` that list `owner_uid` in their
    /// `ownerReferences`, respecting Kubernetes ownership scope rules:
    ///
    /// - Cluster-scoped owner (`owner_namespace` is `""`): scans ALL namespace
    ///   buckets, since cluster-scoped resources can own dependents in any namespace.
    /// - Namespaced owner: scans ONLY the owner's namespace, since namespaced
    ///   resources cannot own cluster-scoped resources or resources in other namespaces.
    fn find_dependents_locked(
        objects: &ObjectStorage,
        owner_uid: &str,
        owner_namespace: &str,
    ) -> Vec<(GVR, String, String)> {
        let mut dependents = Vec::new();

        for (gvr, namespaces) in objects {
            if owner_namespace.is_empty() {
                for (ns, ns_objects) in namespaces {
                    for (name, stored) in ns_objects {
                        if stored
                            .metadata
                            .owner_references
                            .as_ref()
                            .is_some_and(|refs| refs.iter().any(|r| r.uid == owner_uid))
                        {
                            dependents.push((gvr.clone(), ns.clone(), name.clone()));
                        }
                    }
                }
            } else if let Some(ns_objects) = namespaces.get(owner_namespace) {
                for (name, stored) in ns_objects {
                    if stored
                        .metadata
                        .owner_references
                        .as_ref()
                        .is_some_and(|refs| refs.iter().any(|r| r.uid == owner_uid))
                    {
                        dependents.push((gvr.clone(), owner_namespace.to_string(), name.clone()));
                    }
                }
            }
        }

        dependents
    }

    pub fn list(&self, gvr: &GVR, namespace: Option<&str>) -> Result<Vec<Value>> {
        trace!("Listing objects: {:?} in namespace: {:?}", gvr, namespace);

        let objects = self.objects.read().expect("lock poisoned");

        // If no objects of this type exist, return empty list (matches Kubernetes API behavior)
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
        object
            .get("metadata")
            .ok_or_else(|| Error::MetadataError("Object missing metadata field".to_string()))
            .and_then(|meta_value| {
                serde_json::from_value(meta_value.clone())
                    .map_err(|e| Error::MetadataError(format!("Failed to parse metadata: {e}")))
            })
    }
}

impl Default for ObjectTracker {
    /// Creates a tracker with an empty registry. CRD namespace-scope validation
    /// will not apply to any custom resources. Prefer constructing via
    /// `FakeClient` or `ClientBuilder` to share the registry.
    fn default() -> Self {
        Self::new(Arc::new(ResourceRegistry::new()))
    }
}
