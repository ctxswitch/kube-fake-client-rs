use crate::discovery::Discovery;
use crate::generation;
use crate::managed_fields;
use crate::registry::ResourceRegistry;
use crate::scale::ScaleSubresource;
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

const ORPHAN_FINALIZER: &str = "orphan";
const FOREGROUND_FINALIZER: &str = "foregroundDeletion";

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

/// What a delete request asks for the object's dependents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletePropagation {
    /// Neither `propagationPolicy` nor `orphanDependents` is set.
    Unspecified,
    /// `propagationPolicy: Orphan` or `orphanDependents: true`.
    Orphan,
    /// `propagationPolicy: Background` or `Foreground`, or `orphanDependents: false`.
    Cascade,
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

fn has_orphan_finalizer(meta: &ObjectMeta) -> bool {
    meta.finalizers
        .iter()
        .flatten()
        .any(|finalizer| finalizer == ORPHAN_FINALIZER)
}

/// Whether a delete orphans the dependents of the object with `meta`, decided
/// in the order of kube-apiserver's `shouldOrphanDependents`.
fn should_orphan(propagation: DeletePropagation, gvr: &GVR, meta: &ObjectMeta) -> bool {
    match propagation {
        DeletePropagation::Orphan => true,
        DeletePropagation::Cascade => false,
        DeletePropagation::Unspecified => meta
            .finalizers
            .iter()
            .flatten()
            .find_map(|finalizer| match finalizer.as_str() {
                ORPHAN_FINALIZER => Some(true),
                FOREGROUND_FINALIZER => Some(false),
                _ => None,
            })
            .unwrap_or_else(|| orphans_by_default(gvr)),
    }
}

/// The served resources whose `DefaultGarbageCollectionPolicy` is
/// `OrphanDependents`, kept in kube-apiserver for backwards compatibility.
fn orphans_by_default(gvr: &GVR) -> bool {
    matches!(
        (
            gvr.group.as_str(),
            gvr.version.as_str(),
            gvr.resource.as_str()
        ),
        ("batch", "v1", "jobs") | ("", "v1", "replicationcontrollers")
    )
}

pub struct ObjectTracker {
    objects: Arc<RwLock<ObjectStorage>>,
    with_status_subresource: Arc<RwLock<std::collections::HashSet<GVK>>>,
    with_scale_subresource: Arc<RwLock<HashMap<GVK, ScaleSubresource>>>,
    resource_version: Arc<AtomicU64>,
    registry: Arc<ResourceRegistry>,
}

impl ObjectTracker {
    pub fn new(registry: Arc<ResourceRegistry>) -> Self {
        Self {
            objects: Arc::new(RwLock::new(HashMap::new())),
            with_status_subresource: Arc::new(RwLock::new(std::collections::HashSet::new())),
            with_scale_subresource: Arc::new(RwLock::new(HashMap::new())),
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

    pub(crate) fn add_scale_subresource(&self, gvk: GVK, scale: ScaleSubresource) {
        self.with_scale_subresource
            .write()
            .expect("lock poisoned")
            .insert(gvk, scale);
    }

    /// The scale subresource of `gvk`: one registered with
    /// `add_scale_subresource`, or that of a built-in kind discovery lists with
    /// one.
    pub(crate) fn scale_subresource(&self, gvk: &GVK) -> Option<ScaleSubresource> {
        self.with_scale_subresource
            .read()
            .expect("lock poisoned")
            .get(gvk)
            .cloned()
            .or_else(|| {
                Discovery::has_subresource(gvk, "scale")
                    .then(|| ScaleSubresource::built_in(&gvk.kind))
            })
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
        if meta.generation.is_none() && generation::sets_on_create(gvk) {
            meta.generation = Some(1);
        }
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

        if generation::sets_on_create(gvk) {
            meta.generation = Some(1);
        } else if let Some(negative) = meta.generation.filter(|g| *g < 0) {
            return Err(Error::InvalidRequest(format!(
                "{} \"{name}\" is invalid: metadata.generation: Invalid value: {negative}: must be greater than or equal to 0",
                gvk.kind
            )));
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

        // A create cannot set status, apart from a Node's, as kubelets register with one.
        if self.has_status_subresource(gvk) && !(gvk.group.is_empty() && gvk.kind == "Node") {
            if let Some(fields) = object.as_object_mut() {
                fields.remove("status");
            }
        }

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
        object: Value,
        namespace: &str,
        is_status: bool,
    ) -> Result<Value> {
        self.store_update(gvr, gvk, object, namespace, is_status, false)
    }

    /// Updates status from a server-side apply, the one status write whose
    /// `managedFields` count: it recorded them on `object`.
    pub(crate) fn update_applied_status(
        &self,
        gvr: &GVR,
        gvk: &GVK,
        object: Value,
        namespace: &str,
    ) -> Result<Value> {
        self.store_update(gvr, gvk, object, namespace, true, true)
    }

    fn store_update(
        &self,
        gvr: &GVR,
        gvk: &GVK,
        mut object: Value,
        namespace: &str,
        is_status: bool,
        is_status_apply: bool,
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
                // A status update changes only status: kube-apiserver ignores
                // `managedFields` on a subresource request.
                let mut stored = existing.clone();
                copy_status_from(&object, &mut stored);
                if is_status_apply {
                    stored["metadata"]["managedFields"] =
                        object["metadata"]["managedFields"].clone();
                }
                object = stored;
            } else {
                copy_status_from(&existing, &mut object);
            }
        }

        let mut new_meta = self.extract_metadata(&object)?;
        let bumps_generation = !is_status
            && generation::bumps_on_update(gvk, &existing, &existing_meta, &object, &new_meta);
        if !is_status {
            new_meta.managed_fields = match new_meta.managed_fields.take() {
                None => existing_meta.managed_fields,
                Some(entries) if managed_fields::is_reset(&entries) => None,
                entries => entries,
            };
        }
        new_meta.resource_version = existing_meta.resource_version;
        new_meta.uid = existing_meta.uid;
        new_meta.creation_timestamp = existing_meta.creation_timestamp;
        new_meta.namespace = new_meta.namespace.or(existing_meta.namespace);
        new_meta.deletion_grace_period_seconds = new_meta
            .deletion_grace_period_seconds
            .or(existing_meta.deletion_grace_period_seconds);

        new_meta.generation = if bumps_generation {
            Some(increment_generation(existing_meta.generation))
        } else {
            existing_meta.generation
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

        // Like kube-apiserver's storage layer, skip a write that changes nothing.
        if managed_fields::unchanged(&object, &existing) {
            return Ok(existing);
        }

        new_meta.resource_version = Some(self.next_resource_version());
        object["metadata"] = serde_json::to_value(&new_meta)?;

        // The last finalizer was removed. The stored copy still carries the old
        // finalizers, so remove it directly rather than through `delete`. Like
        // kube-apiserver, return the request object. A terminating Namespace
        // that still holds objects stays: kube-apiserver keeps it on
        // `spec.finalizers` until the namespace controller has emptied it.
        if should_be_deleted(&new_meta) {
            let mut objects = self.objects.write().expect("lock poisoned");
            if !(*gvr == namespaces_gvr() && Self::namespace_has_content_locked(&objects, &name)) {
                self.remove_locked(&mut objects, gvr, namespace, &name)?;
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

    /// Deletes `name`, playing both kube-apiserver and the garbage collector.
    ///
    /// When the delete orphans (see [`should_orphan`]), the `orphan` finalizer
    /// is added for the response, and [`Self::finish_orphaning_locked`] clears
    /// it again before returning, without waiting for other finalizers.
    ///
    /// An object with finalizers is not removed. It is marked as terminating
    /// (see [`Self::mark_for_deletion`]) and returned, and any dependents it
    /// still has stay until an update removes its last finalizer.
    ///
    /// Otherwise the object is removed with its dependents (see
    /// [`Self::remove_locked`]).
    ///
    /// A Namespace follows [`Self::delete_namespace_locked`] instead.
    pub fn delete(
        &self,
        gvr: &GVR,
        namespace: &str,
        name: &str,
        propagation: DeletePropagation,
    ) -> Result<Value> {
        trace!("Deleting object: {:?} {}/{}", gvr, namespace, name);

        let mut objects = self.objects.write().expect("lock poisoned");

        if *gvr == namespaces_gvr() {
            return self.delete_namespace_locked(&mut objects, name);
        }
        self.delete_locked(&mut objects, gvr, namespace, name, propagation)
    }

    /// Deletes one object under the caller's write lock: sets the `orphan`
    /// finalizer as `propagation` decides, then marks the object when it has
    /// finalizers and removes it otherwise. The response keeps `orphan`, as
    /// kube-apiserver's does, because the garbage collector clears it later.
    fn delete_locked(
        &self,
        objects: &mut ObjectStorage,
        gvr: &GVR,
        namespace: &str,
        name: &str,
        propagation: DeletePropagation,
    ) -> Result<Value> {
        let stored = objects
            .get_mut(gvr)
            .and_then(|gvr_objects| gvr_objects.get_mut(namespace))
            .and_then(|ns_objects| ns_objects.get_mut(name))
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;

        let orphan = should_orphan(propagation, gvr, &stored.metadata);
        self.set_orphan_finalizer(stored, orphan)?;

        if stored
            .metadata
            .finalizers
            .as_ref()
            .is_none_or(Vec::is_empty)
        {
            return self.remove_locked(objects, gvr, namespace, name);
        }

        self.mark_for_deletion(stored)?;
        debug!("Marked object for deletion: {}/{}", namespace, name);
        let marked = stored.data.clone();
        if orphan {
            self.finish_orphaning_locked(objects, gvr, namespace, name)?;
        }
        Ok(marked)
    }

    /// Does the garbage collector's orphaning for a terminating object: removes
    /// it from its dependents' `ownerReferences`, then removes the `orphan`
    /// finalizer, and removes the object if that was the last finalizer.
    fn finish_orphaning_locked(
        &self,
        objects: &mut ObjectStorage,
        gvr: &GVR,
        namespace: &str,
        name: &str,
    ) -> Result<()> {
        let uid = objects
            .get(gvr)
            .and_then(|gvr_objects| gvr_objects.get(namespace))
            .and_then(|ns_objects| ns_objects.get(name))
            .ok_or_else(|| gvr.not_found_error(namespace, name))?
            .metadata
            .uid
            .clone();

        if let Some(uid) = uid {
            for (dep_gvr, dep_ns, dep_name) in
                Self::find_dependents_locked(objects, &uid, namespace)
            {
                let dependent = objects
                    .get_mut(&dep_gvr)
                    .and_then(|g| g.get_mut(&dep_ns))
                    .and_then(|n| n.get_mut(&dep_name))
                    .ok_or_else(|| dep_gvr.not_found_error(&dep_ns, &dep_name))?;
                self.remove_owner_reference(dependent, &uid)?;
                debug!("Orphaned dependent: {}/{}", dep_ns, dep_name);
            }
        }

        // Fetched after the loop: an object can be its own dependent.
        let stored = objects
            .get_mut(gvr)
            .and_then(|gvr_objects| gvr_objects.get_mut(namespace))
            .and_then(|ns_objects| ns_objects.get_mut(name))
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;
        self.set_orphan_finalizer(stored, false)?;

        if should_be_deleted(&stored.metadata) {
            self.remove_locked(objects, gvr, namespace, name)?;
        }
        Ok(())
    }

    /// Adds or removes the `orphan` finalizer. An `orphan` that is already
    /// present keeps its position.
    fn set_orphan_finalizer(&self, stored: &mut StoredObject, present: bool) -> Result<()> {
        if has_orphan_finalizer(&stored.metadata) == present {
            return Ok(());
        }

        let finalizers = stored.metadata.finalizers.get_or_insert_default();
        if present {
            finalizers.push(ORPHAN_FINALIZER.to_string());
        } else {
            finalizers.retain(|finalizer| finalizer != ORPHAN_FINALIZER);
        }
        if finalizers.is_empty() {
            stored.metadata.finalizers = None;
        }
        self.commit_metadata(stored)
    }

    fn remove_owner_reference(&self, stored: &mut StoredObject, owner_uid: &str) -> Result<()> {
        let remaining: Vec<_> = stored
            .metadata
            .owner_references
            .take()
            .unwrap_or_default()
            .into_iter()
            .filter(|reference| reference.uid != owner_uid)
            .collect();
        stored.metadata.owner_references = (!remaining.is_empty()).then_some(remaining);
        self.commit_metadata(stored)
    }

    /// Publishes a change to `stored.metadata`: `get` and `list` read `data`.
    fn commit_metadata(&self, stored: &mut StoredObject) -> Result<()> {
        stored.metadata.resource_version = Some(self.next_resource_version());
        stored.data["metadata"] = serde_json::to_value(&stored.metadata)?;
        Ok(())
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
                    self.delete_locked(
                        objects,
                        &obj_gvr,
                        name,
                        &obj_name,
                        DeletePropagation::Cascade,
                    )?;
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
        self.delete_locked(objects, &gvr, "", name, DeletePropagation::Unspecified)
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
            self.remove_locked(objects, &gvr, "", name)?;
            debug!("Removed finalized namespace: {}", name);
        }
        Ok(())
    }

    /// Marks `stored` as terminating the way kube-apiserver's `markAsDeleting`
    /// does: bumps `generation` on the first mark, sets `deletionTimestamp` to
    /// now unless an earlier one is set, and sets `deletionGracePeriodSeconds`
    /// to 0. The resource version changes only when the metadata did.
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
            self.commit_metadata(stored)?;
        }
        Ok(())
    }

    /// Removes `name` from `objects` and its dependents, under the caller's
    /// write lock. A terminating Namespace emptied by the removal is removed
    /// too.
    ///
    /// An iterative worklist collects the object's UID, scans for dependents
    /// respecting Kubernetes scope rules (cluster-scoped owners scan all
    /// namespaces, namespaced owners scan only their own namespace), removes
    /// them, collects their UIDs, and repeats until no more dependents are
    /// found. A visited set prevents cycles. A dependent with finalizers is
    /// marked instead of removed, and its own dependents stay until it goes,
    /// unless it carries `orphan`, which orphans them now.
    fn remove_locked(
        &self,
        objects: &mut ObjectStorage,
        gvr: &GVR,
        namespace: &str,
        name: &str,
    ) -> Result<Value> {
        let stored = objects
            .get_mut(gvr)
            .and_then(|gvr_objects| gvr_objects.get_mut(namespace))
            .and_then(|ns_objects| ns_objects.remove(name))
            .ok_or_else(|| gvr.not_found_error(namespace, name))?;

        debug!("Deleted object: {}/{}", namespace, name);
        let data = stored.data;

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
                        if has_orphan_finalizer(&dep_stored.metadata) {
                            self.finish_orphaning_locked(objects, &dep_gvr, &dep_ns, &dep_name)?;
                        }
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
