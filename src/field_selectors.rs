//! Field selector support for filtering Kubernetes resources
//!
//! This module provides support for Kubernetes field selectors, including
//! pre-registered fields that work without requiring custom indexes.

use serde_json::Value;

/// Helper to extract a string field at a given path (e.g., "spec", "nodeName")
fn get_string_field(obj: &Value, parent: &str, field: &str) -> Option<Vec<String>> {
    obj.get(parent)
        .and_then(|p| p.get(field))
        .and_then(|v| v.as_str())
        .map(|s| vec![s.to_string()])
}

/// Helper to extract a boolean field at a given path
fn get_bool_field(obj: &Value, parent: &str, field: &str) -> Option<Vec<String>> {
    obj.get(parent)
        .and_then(|p| p.get(field))
        .and_then(|v| v.as_bool())
        .map(|b| vec![b.to_string()])
}

/// Helper to extract an integer field at a given path
fn get_int_field(obj: &Value, parent: &str, field: &str) -> Option<Vec<String>> {
    obj.get(parent)
        .and_then(|p| p.get(field))
        .and_then(|v| v.as_i64())
        .map(|n| vec![n.to_string()])
}

/// Helper to extract a top-level string field
fn get_top_level_string(obj: &Value, field: &str) -> Option<Vec<String>> {
    obj.get(field)
        .and_then(|v| v.as_str())
        .map(|s| vec![s.to_string()])
}

/// Extract value from pre-registered field paths that are supported by Kubernetes
/// without requiring an index. Based on official Kubernetes documentation:
/// <https://kubernetes.io/docs/concepts/overview/working-with-objects/field-selectors/>
///
/// Universal fields (all resources):
/// - metadata.name
/// - metadata.namespace
///
/// Resource-specific pre-registered fields are automatically supported based on the Kind.
pub fn extract_preregistered_field_value(
    obj_value: &Value,
    field: &str,
    kind: &str,
) -> Option<Vec<String>> {
    // Universal metadata fields (supported by all resources)
    match field {
        "metadata.name" => return get_string_field(obj_value, "metadata", "name"),
        "metadata.namespace" => return get_string_field(obj_value, "metadata", "namespace"),
        _ => {}
    }

    // Resource-specific pre-registered fields
    match kind {
        "Pod" => match field {
            "spec.nodeName" => get_string_field(obj_value, "spec", "nodeName"),
            "spec.restartPolicy" => get_string_field(obj_value, "spec", "restartPolicy"),
            "spec.schedulerName" => get_string_field(obj_value, "spec", "schedulerName"),
            "spec.serviceAccountName" => get_string_field(obj_value, "spec", "serviceAccountName"),
            "spec.hostNetwork" => get_bool_field(obj_value, "spec", "hostNetwork"),
            "status.phase" => get_string_field(obj_value, "status", "phase"),
            "status.podIP" => get_string_field(obj_value, "status", "podIP"),
            "status.nominatedNodeName" => {
                get_string_field(obj_value, "status", "nominatedNodeName")
            }
            _ => None,
        },
        "Event" => match field {
            "involvedObject.kind" => get_string_field(obj_value, "involvedObject", "kind"),
            "involvedObject.namespace" => {
                get_string_field(obj_value, "involvedObject", "namespace")
            }
            "involvedObject.name" => get_string_field(obj_value, "involvedObject", "name"),
            "involvedObject.uid" => get_string_field(obj_value, "involvedObject", "uid"),
            "involvedObject.apiVersion" => {
                get_string_field(obj_value, "involvedObject", "apiVersion")
            }
            "involvedObject.resourceVersion" => {
                get_string_field(obj_value, "involvedObject", "resourceVersion")
            }
            "involvedObject.fieldPath" => {
                get_string_field(obj_value, "involvedObject", "fieldPath")
            }
            "reason" => get_top_level_string(obj_value, "reason"),
            "reportingComponent" => get_top_level_string(obj_value, "reportingComponent"),
            "source" => get_top_level_string(obj_value, "source"),
            "type" => get_top_level_string(obj_value, "type"),
            _ => None,
        },
        "Secret" => match field {
            "type" => get_top_level_string(obj_value, "type"),
            _ => None,
        },
        "Namespace" => match field {
            "status.phase" => get_string_field(obj_value, "status", "phase"),
            _ => None,
        },
        "ReplicaSet" | "ReplicationController" => match field {
            "status.replicas" => get_int_field(obj_value, "status", "replicas"),
            _ => None,
        },
        "Job" => match field {
            "status.successful" => get_int_field(obj_value, "status", "successful"),
            _ => None,
        },
        "Node" => match field {
            "spec.unschedulable" => get_bool_field(obj_value, "spec", "unschedulable"),
            _ => None,
        },
        "CertificateSigningRequest" => match field {
            "spec.signerName" => get_string_field(obj_value, "spec", "signerName"),
            _ => None,
        },
        _ => None,
    }
}
