//! Which kinds use `metadata.generation`, and which updates bump it.
//!
//! kube-apiserver decides this per kind, in each resource strategy's
//! `PrepareForCreate` and `PrepareForUpdate`. Kinds with neither leave it unset.

use crate::discovery::Discovery;
use crate::tracker::GVK;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use serde_json::Value;
use std::collections::BTreeMap;

/// Built-in kinds whose strategy sets `generation` to 1 on create.
const SETS_ON_CREATE: &[(&str, &str)] = &[
    ("", "Pod"),
    ("", "PodTemplate"),
    ("", "ReplicationController"),
    ("admissionregistration.k8s.io", "MutatingAdmissionPolicy"),
    (
        "admissionregistration.k8s.io",
        "MutatingAdmissionPolicyBinding",
    ),
    (
        "admissionregistration.k8s.io",
        "MutatingWebhookConfiguration",
    ),
    ("admissionregistration.k8s.io", "ValidatingAdmissionPolicy"),
    (
        "admissionregistration.k8s.io",
        "ValidatingAdmissionPolicyBinding",
    ),
    (
        "admissionregistration.k8s.io",
        "ValidatingWebhookConfiguration",
    ),
    ("apiextensions.k8s.io", "CustomResourceDefinition"),
    ("apps", "DaemonSet"),
    ("apps", "Deployment"),
    ("apps", "ReplicaSet"),
    ("apps", "StatefulSet"),
    ("batch", "CronJob"),
    ("batch", "Job"),
    ("discovery.k8s.io", "EndpointSlice"),
    ("flowcontrol.apiserver.k8s.io", "FlowSchema"),
    ("flowcontrol.apiserver.k8s.io", "PriorityLevelConfiguration"),
    ("networking.k8s.io", "Ingress"),
    ("networking.k8s.io", "IngressClass"),
    ("networking.k8s.io", "NetworkPolicy"),
    ("policy", "PodDisruptionBudget"),
    ("resource.k8s.io", "DeviceClass"),
    ("resource.k8s.io", "DeviceTaintRule"),
    ("resource.k8s.io", "ResourceSlice"),
    ("scheduling.k8s.io", "PriorityClass"),
];

fn is_kind(gvk: &GVK, group: &str, kind: &str) -> bool {
    gvk.group == group && gvk.kind == kind
}

/// Returns whether creating `gvk` sets `generation` to 1. A kind unknown to
/// discovery is a custom resource, which always does.
pub(crate) fn sets_on_create(gvk: &GVK) -> bool {
    Discovery::is_namespaced(gvk).is_none()
        || SETS_ON_CREATE
            .iter()
            .any(|(group, kind)| is_kind(gvk, group, kind))
}

fn without_metadata(object: &Value) -> Value {
    let mut object = object.clone();
    if let Some(fields) = object.as_object_mut() {
        fields.remove("metadata");
    }
    object
}

/// Compares maps the way `apiequality.Semantic.DeepEqual` does: empty equals absent.
fn map_changed(
    old: &Option<BTreeMap<String, String>>,
    new: &Option<BTreeMap<String, String>>,
) -> bool {
    old.as_ref().filter(|map| !map.is_empty()) != new.as_ref().filter(|map| !map.is_empty())
}

/// Returns whether replacing `old` with `new` bumps `generation`.
///
/// Upstream strategies compare a named field, such as `spec` or `webhooks`.
/// Comparing everything outside `metadata` is equivalent for each kind here,
/// provided the caller has already reset `status` on a kind with a status
/// subresource: no such kind has another field outside the one compared.
pub(crate) fn bumps_on_update(
    gvk: &GVK,
    old: &Value,
    old_meta: &ObjectMeta,
    new: &Value,
    new_meta: &ObjectMeta,
) -> bool {
    let tracked = sets_on_create(gvk) || is_kind(gvk, "storage.k8s.io", "CSIDriver");
    if !tracked || is_kind(gvk, "scheduling.k8s.io", "PriorityClass") {
        return false;
    }
    without_metadata(old) != without_metadata(new)
        || (is_kind(gvk, "apps", "Deployment")
            && map_changed(&old_meta.annotations, &new_meta.annotations))
        || (is_kind(gvk, "discovery.k8s.io", "EndpointSlice")
            && map_changed(&old_meta.labels, &new_meta.labels))
}
