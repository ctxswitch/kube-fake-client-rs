//! Field selector support for filtering Kubernetes resources
//!
//! This module provides support for Kubernetes field selectors, including
//! pre-registered fields that work without requiring custom indexes.

use serde_json::Value;

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
        "metadata.name" => {
            return obj_value
                .get("metadata")
                .and_then(|m| m.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]);
        }
        "metadata.namespace" => {
            return obj_value
                .get("metadata")
                .and_then(|m| m.get("namespace"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]);
        }
        _ => {}
    }

    // Resource-specific pre-registered fields
    match kind {
        "Pod" => match field {
            "spec.nodeName" => obj_value
                .get("spec")
                .and_then(|s| s.get("nodeName"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]),
            "spec.restartPolicy" => obj_value
                .get("spec")
                .and_then(|s| s.get("restartPolicy"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]),
            "spec.schedulerName" => obj_value
                .get("spec")
                .and_then(|s| s.get("schedulerName"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]),
            "spec.serviceAccountName" => obj_value
                .get("spec")
                .and_then(|s| s.get("serviceAccountName"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]),
            "spec.hostNetwork" => obj_value
                .get("spec")
                .and_then(|s| s.get("hostNetwork"))
                .and_then(|b| b.as_bool())
                .map(|b| vec![b.to_string()]),
            "status.phase" => obj_value
                .get("status")
                .and_then(|s| s.get("phase"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]),
            "status.podIP" => obj_value
                .get("status")
                .and_then(|s| s.get("podIP"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]),
            "status.nominatedNodeName" => obj_value
                .get("status")
                .and_then(|s| s.get("nominatedNodeName"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]),
            _ => None,
        },
        "Event" => match field {
            "involvedObject.kind" => obj_value
                .get("involvedObject")
                .and_then(|o| o.get("kind"))
                .and_then(|k| k.as_str())
                .map(|s| vec![s.to_string()]),
            "involvedObject.namespace" => obj_value
                .get("involvedObject")
                .and_then(|o| o.get("namespace"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]),
            "involvedObject.name" => obj_value
                .get("involvedObject")
                .and_then(|o| o.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]),
            "involvedObject.uid" => obj_value
                .get("involvedObject")
                .and_then(|o| o.get("uid"))
                .and_then(|u| u.as_str())
                .map(|s| vec![s.to_string()]),
            "involvedObject.apiVersion" => obj_value
                .get("involvedObject")
                .and_then(|o| o.get("apiVersion"))
                .and_then(|a| a.as_str())
                .map(|s| vec![s.to_string()]),
            "involvedObject.resourceVersion" => obj_value
                .get("involvedObject")
                .and_then(|o| o.get("resourceVersion"))
                .and_then(|r| r.as_str())
                .map(|s| vec![s.to_string()]),
            "involvedObject.fieldPath" => obj_value
                .get("involvedObject")
                .and_then(|o| o.get("fieldPath"))
                .and_then(|f| f.as_str())
                .map(|s| vec![s.to_string()]),
            "reason" => obj_value
                .get("reason")
                .and_then(|r| r.as_str())
                .map(|s| vec![s.to_string()]),
            "reportingComponent" => obj_value
                .get("reportingComponent")
                .and_then(|r| r.as_str())
                .map(|s| vec![s.to_string()]),
            "source" => obj_value
                .get("source")
                .and_then(|s| s.as_str())
                .map(|s| vec![s.to_string()]),
            "type" => obj_value
                .get("type")
                .and_then(|t| t.as_str())
                .map(|s| vec![s.to_string()]),
            _ => None,
        },
        "Secret" => match field {
            "type" => obj_value
                .get("type")
                .and_then(|t| t.as_str())
                .map(|s| vec![s.to_string()]),
            _ => None,
        },
        "Namespace" => match field {
            "status.phase" => obj_value
                .get("status")
                .and_then(|s| s.get("phase"))
                .and_then(|p| p.as_str())
                .map(|s| vec![s.to_string()]),
            _ => None,
        },
        "ReplicaSet" | "ReplicationController" => match field {
            "status.replicas" => obj_value
                .get("status")
                .and_then(|s| s.get("replicas"))
                .and_then(|r| r.as_i64())
                .map(|n| vec![n.to_string()]),
            _ => None,
        },
        "Job" => match field {
            "status.successful" => obj_value
                .get("status")
                .and_then(|s| s.get("successful"))
                .and_then(|r| r.as_i64())
                .map(|n| vec![n.to_string()]),
            _ => None,
        },
        "Node" => match field {
            "spec.unschedulable" => obj_value
                .get("spec")
                .and_then(|s| s.get("unschedulable"))
                .and_then(|b| b.as_bool())
                .map(|b| vec![b.to_string()]),
            _ => None,
        },
        "CertificateSigningRequest" => match field {
            "spec.signerName" => obj_value
                .get("spec")
                .and_then(|s| s.get("signerName"))
                .and_then(|n| n.as_str())
                .map(|s| vec![s.to_string()]),
            _ => None,
        },
        _ => None,
    }
}
