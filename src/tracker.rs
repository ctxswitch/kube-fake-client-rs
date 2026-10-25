use crate::discovery::Discovery;
use crate::registry::ResourceRegistry;
use crate::utils::{
    deletion_timestamp_equal, ensure_metadata, increment_generation, should_be_deleted,
};
use crate::{Error, Result};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
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

/// Sets `to`'s status to `from`'s, removing it when `from` has none.
fn copy_status_from(from: &Value, to: &mut Value) {
    match from.get("status") {
        Some(status) => to["status"] = status.clone(),
        None => {
            if let Some(fields) = to.as_object_mut() {
                fields.remove("status");
            }
        }
    }
}

fn namespaces_gvr() -> GVR {
    GVR::new("", "v1", "namespaces")
}

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

    /// Built-in kinds get their status subresource from discovery, as
    /// controller-runtime's fake client does for in-tree kinds; other kinds
    /// need `add_status_subresource`.
    pub fn has_status_subresource(&self, gvk: &GVK) -> bool {
        Discovery::has_subresource(gvk, "status")
            || self
                .with_status_subresource
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

        // kube-apiserver's NamespaceLifecycle admission. `add` skips it, as
        // seeding bypasses admission.
        if !namespace.is_empty()
            && self
                .get(&namespaces_gvr(), "", namespace)
                .is_ok_and(|ns| ns["status"]["phase"] == "Terminating")
        {
            return Err(Error::Forbidden {
                kind: gvr.resource.clone(),
                group: gvr.group.clone(),
                name,
                message: format!(
                    "unable to create new content in namespace {namespace} because it is being terminated"
                ),
                causes: vec![kube::core::response::StatusCause {
                    reason: "NamespaceTerminating".to_string(),
                    message: format!("namespace {namespace} is being terminated"),
                    field: "metadata.namespace".to_string(),
                }],
            });
        }

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

        // Split spec and status as controller-runtime's fake client does
        if self.has_status_subresource(gvk) {
            if is_status {
                // A status update changes only status. The managed fields an
                // apply recorded on the request are kept with it; a request
                // without them keeps the stored ones.
                let mut stored = existing.clone();
                copy_status_from(&object, &mut stored);
                if let Some(managed) = object["metadata"].get("managedFields") {
                    stored["metadata"]["managedFields"] = managed.clone();
                }
                object = stored;
            } else {
                copy_status_from(&existing, &mut object);
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

        if existing_meta.deletion_timestamp.is_some() {
            let existing_finalizers = existing_meta.finalizers.as_deref().unwrap_or_default();
            let added: BTreeSet<&String> = new_meta
                .finalizers
                .iter()
                .flatten()
                .filter(|f| !existing_finalizers.contains(f))
                .collect();
            if !added.is_empty() {
                let added: Vec<String> = added.iter().map(|f| format!("{f:?}")).collect();
                return Err(Error::InvalidRequest(format!(
                    "metadata.finalizers: Forbidden: no new finalizers can be added if the object is being deleted, found new finalizers []string{{{}}}",
                    added.join(", ")
                )));
            }
        }

        object["metadata"] = serde_json::to_value(&new_meta)?;

        // The last finalizer was removed. The stored copy still carries the old
        // finalizers, so remove it directly rather than through `delete`. Like
        // kube-apiserver, return the request object. A terminating Namespace
        // that still holds objects stays: kube-apiserver keeps it on
        // `spec.finalizers` until the namespace controller has emptied it.
        if should_be_deleted(&new_meta) {
            let mut objects = self.objects.write().expect("lock poisoned");
            if !(*gvr == namespaces_gvr() && Self::namespace_has_content_locked(&objects, &name)) {
                self.remove_locked(&mut objects, gvr, namespace, &name, true)?;
                return Ok(object);
            }
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
    /// An object with finalizers is not removed. It is marked as terminating
    /// (see [`Self::mark_for_deletion`]) and returned, and its dependents stay
    /// until an update removes its last finalizer.
    ///
    /// Otherwise the object is removed. When `cascade` is `true`, an iterative
    /// worklist under a single write lock collects its UID, scans for dependents
    /// respecting Kubernetes scope rules (cluster-scoped owners scan all
    /// namespaces, namespaced owners scan only their own namespace), removes
    /// them, collects their UIDs, and repeats until no more dependents are found.
    /// A visited set prevents cycles. A dependent with finalizers is marked
    /// instead of removed, and its own dependents are left in place.
    ///
    /// When `cascade` is `false`, only the target object is removed — no
    /// dependent scanning or deletion is performed.
    ///
    /// A Namespace follows [`Self::delete_namespace_locked`] instead.
    pub fn delete(&self, gvr: &GVR, namespace: &str, name: &str, cascade: bool) -> Result<Value> {
        trace!("Deleting object: {:?} {}/{}", gvr, namespace, name);

        let mut objects = self.objects.write().expect("lock poisoned");

        if *gvr == namespaces_gvr() {
            return self.delete_namespace_locked(&mut objects, name);
        }
        self.delete_locked(&mut objects, gvr, namespace, name, cascade)
    }

    /// Deletes one object under the caller's write lock: marks it when it has
    /// finalizers, and otherwise removes it.
    fn delete_locked(
        &self,
        objects: &mut ObjectStorage,
        gvr: &GVR,
        namespace: &str,
        name: &str,
        cascade: bool,
    ) -> Result<Value> {
        let stored = objects
            .get_mut(gvr)
            .and_then(|gvr_objects| gvr_objects.get_mut(namespace))
            .and_then(|ns_objects| ns_objects.get_mut(name))
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;

        if stored
            .metadata
            .finalizers
            .as_ref()
            .is_some_and(|f| !f.is_empty())
        {
            self.mark_for_deletion(stored)?;
            debug!("Marked object for deletion: {}/{}", namespace, name);
            return Ok(stored.data.clone());
        }

        self.remove_locked(objects, gvr, namespace, name, cascade)
    }

    /// Deletes Namespace `name` the way kube-apiserver and the namespace
    /// controller do together. The first delete sets `deletionTimestamp` and
    /// `status.phase: Terminating`, then deletes every object in the namespace
    /// with background propagation. The Namespace leaves the store once it
    /// holds no objects and has no finalizers (see
    /// [`Self::remove_namespace_if_finalized_locked`]). The fake does not show
    /// the `kubernetes` entry in `spec.finalizers`; remaining content plays its
    /// part.
    fn delete_namespace_locked(&self, objects: &mut ObjectStorage, name: &str) -> Result<Value> {
        let gvr = namespaces_gvr();
        let stored = objects
            .get_mut(&gvr)
            .and_then(|gvr_objects| gvr_objects.get_mut(""))
            .and_then(|ns_objects| ns_objects.get_mut(name))
            .ok_or_else(|| gvr.not_found_error("", name))?;

        if stored.metadata.deletion_timestamp.is_none() {
            stored.metadata.deletion_timestamp = Some(Time(jiff::Timestamp::now()));
            stored.metadata.resource_version = Some(self.next_resource_version());
            stored.data["metadata"] = serde_json::to_value(&stored.metadata)?;
            stored.data["status"]["phase"] = Value::from("Terminating");
            let marked = stored.data.clone();
            debug!("Marked namespace for deletion: {}", name);

            let contents: Vec<(GVR, String)> = objects
                .iter()
                .filter_map(|(gvr, namespaces)| namespaces.get(name).map(|objs| (gvr, objs)))
                .flat_map(|(gvr, objs)| objs.keys().map(move |obj| (gvr.clone(), obj.clone())))
                .collect();
            for (obj_gvr, obj_name) in contents {
                // An earlier cascade may already have removed it.
                let present = objects
                    .get(&obj_gvr)
                    .and_then(|namespaces| namespaces.get(name))
                    .is_some_and(|objs| objs.contains_key(&obj_name));
                if present {
                    self.delete_locked(objects, &obj_gvr, name, &obj_name, true)?;
                }
            }

            self.remove_namespace_if_finalized_locked(objects, name)?;
            return Ok(marked);
        }

        // While content remains kube-apiserver still sees `spec.finalizers` and
        // returns the Namespace unchanged; after that it falls through to the
        // generic delete.
        if Self::namespace_has_content_locked(objects, name) {
            return objects
                .get(&gvr)
                .and_then(|gvr_objects| gvr_objects.get(""))
                .and_then(|ns_objects| ns_objects.get(name))
                .map(|stored| stored.data.clone())
                .ok_or_else(|| gvr.not_found_error("", name));
        }
        self.delete_locked(objects, &gvr, "", name, true)
    }

    fn namespace_has_content_locked(objects: &ObjectStorage, namespace: &str) -> bool {
        objects.values().any(|namespaces| {
            namespaces
                .get(namespace)
                .is_some_and(|objs| !objs.is_empty())
        })
    }

    /// Removes Namespace `name` once it is terminating, has no finalizers and
    /// holds no objects: the namespace controller's last step.
    fn remove_namespace_if_finalized_locked(
        &self,
        objects: &mut ObjectStorage,
        name: &str,
    ) -> Result<()> {
        let gvr = namespaces_gvr();
        let finalized = objects
            .get(&gvr)
            .and_then(|gvr_objects| gvr_objects.get(""))
            .and_then(|ns_objects| ns_objects.get(name))
            .is_some_and(|stored| should_be_deleted(&stored.metadata));
        if finalized && !Self::namespace_has_content_locked(objects, name) {
            self.remove_locked(objects, &gvr, "", name, true)?;
            debug!("Removed finalized namespace: {}", name);
        }
        Ok(())
    }

    /// Marks `stored` as terminating the way kube-apiserver's `markAsDeleting`
    /// does: bumps `generation` on the first mark, sets `deletionTimestamp` to
    /// now unless an earlier one is set, and sets `deletionGracePeriodSeconds`
    /// to 0. The resource version changes only when the metadata did, so a
    /// repeated delete is a no-op.
    fn mark_for_deletion(&self, stored: &mut StoredObject) -> Result<()> {
        let meta = &mut stored.metadata;
        let now = jiff::Timestamp::now();
        let mut changed = false;

        if meta.deletion_timestamp.is_none() && meta.generation.is_some_and(|g| g > 0) {
            meta.generation = Some(increment_generation(meta.generation));
            changed = true;
        }
        if meta.deletion_timestamp.as_ref().is_none_or(|ts| ts.0 > now) {
            meta.deletion_timestamp = Some(Time(now));
            changed = true;
        }
        if meta.deletion_grace_period_seconds != Some(0) {
            meta.deletion_grace_period_seconds = Some(0);
            changed = true;
        }

        if changed {
            meta.resource_version = Some(self.next_resource_version());
            stored.data["metadata"] = serde_json::to_value(&stored.metadata)?;
        }
        Ok(())
    }

    /// Removes `name` from `objects` and, when `cascade` is `true`, its
    /// dependents. A terminating Namespace emptied by the removal is removed
    /// too. The caller holds the write lock.
    fn remove_locked(
        &self,
        objects: &mut ObjectStorage,
        gvr: &GVR,
        namespace: &str,
        name: &str,
        cascade: bool,
    ) -> Result<Value> {
        let stored = objects
            .get_mut(gvr)
            .and_then(|gvr_objects| gvr_objects.get_mut(namespace))
            .and_then(|ns_objects| ns_objects.remove(name))
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;

        debug!("Deleted object: {}/{}", namespace, name);
        let data = stored.data;

        if !cascade {
            self.remove_namespace_if_finalized_locked(objects, namespace)?;
            return Ok(data);
        }

        let mut touched_namespaces = BTreeSet::from([namespace.to_string()]);
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
            let dependents = Self::find_dependents_locked(objects, &uid, &owner_ns);
            for (dep_gvr, dep_ns, dep_name) in dependents {
                let Some(dep_stored) = objects
                    .get_mut(&dep_gvr)
                    .and_then(|g| g.get_mut(&dep_ns))
                    .and_then(|n| n.get_mut(&dep_name))
                else {
                    continue;
                };
                // A dependent with finalizers stays until they are removed, and one
                // already being deleted is left as it is, as the garbage collector does.
                if dep_stored
                    .metadata
                    .finalizers
                    .as_ref()
                    .is_some_and(|f| !f.is_empty())
                {
                    if dep_stored.metadata.deletion_timestamp.is_none() {
                        self.mark_for_deletion(dep_stored)?;
                        debug!(
                            "Cascade-marked dependent for deletion: {}/{}",
                            dep_ns, dep_name
                        );
                    }
                    continue;
                }
                if let Some(dep_stored) = objects
                    .get_mut(&dep_gvr)
                    .and_then(|g| g.get_mut(&dep_ns))
                    .and_then(|n| n.remove(&dep_name))
                {
                    debug!("Cascade-deleted dependent: {}/{}", dep_ns, dep_name);
                    touched_namespaces.insert(dep_ns.clone());
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

        for ns in &touched_namespaces {
            self.remove_namespace_if_finalized_locked(objects, ns)?;
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
