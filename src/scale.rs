//! The `scale` subresource
//!
//! Converts between a parent object and its `autoscaling/v1` `Scale` the way
//! kube-apiserver's scale storage does.

use crate::{Error, Result};
use serde_json::{json, Map, Value};

/// Where a kind keeps the fields its `Scale` exposes.
#[derive(Debug, Clone)]
pub(crate) struct ScaleSubresource {
    spec_replicas: Vec<String>,
    status_replicas: Vec<String>,
    selector: SelectorSource,
}

#[derive(Debug, Clone)]
enum SelectorSource {
    /// `spec.selector` as a `LabelSelector` (Deployment, ReplicaSet, StatefulSet)
    LabelSelector,
    /// `spec.selector` as a plain label map (ReplicationController)
    Map,
    /// A selector already serialized as a string at this path (custom resources)
    Path(Vec<String>),
    /// No selector (a custom resource without `labelSelectorPath`)
    None,
}

impl ScaleSubresource {
    /// The scale subresource of a built-in kind that has one.
    pub(crate) fn built_in(kind: &str) -> Self {
        let selector = if kind == "ReplicationController" {
            SelectorSource::Map
        } else {
            SelectorSource::LabelSelector
        };
        Self {
            spec_replicas: vec!["spec".to_string(), "replicas".to_string()],
            status_replicas: vec!["status".to_string(), "replicas".to_string()],
            selector,
        }
    }

    /// The scale subresource of a custom resource, with paths in the form of
    /// CRD `subresources.scale` (`.spec.replicas`).
    pub(crate) fn custom(
        spec_replicas_path: &str,
        status_replicas_path: &str,
        label_selector_path: Option<&str>,
    ) -> Self {
        let spec_replicas = parse_path(spec_replicas_path);
        let status_replicas = parse_path(status_replicas_path);
        assert!(
            !spec_replicas.is_empty() && !status_replicas.is_empty(),
            "scale replica paths must name a field, such as .spec.replicas"
        );
        Self {
            spec_replicas,
            status_replicas,
            selector: label_selector_path.map_or(SelectorSource::None, |path| {
                SelectorSource::Path(parse_path(path))
            }),
        }
    }
}

/// Splits a dotted JSON path (`.spec.replicas`) into its fields. CRD scale
/// paths have no array notation.
fn parse_path(path: &str) -> Vec<String> {
    path.split('.')
        .filter(|field| !field.is_empty())
        .map(str::to_string)
        .collect()
}

/// Builds the `Scale` for `parent`. Missing replica counts read as 0, and
/// `spec.replicas` and `status.selector` are left out when empty, as they are
/// `omitempty` in `autoscaling/v1`.
pub(crate) fn to_scale(parent: &Value, scale: &ScaleSubresource) -> Result<Value> {
    let mut metadata = Map::new();
    for field in [
        "name",
        "namespace",
        "uid",
        "resourceVersion",
        "creationTimestamp",
    ] {
        if let Some(value) = parent["metadata"].get(field) {
            metadata.insert(field.to_string(), value.clone());
        }
    }

    let selector = match &scale.selector {
        SelectorSource::LabelSelector => selector_string(&parent["spec"]["selector"])?,
        SelectorSource::Map => map_selector_string(&parent["spec"]["selector"]),
        SelectorSource::Path(path) => value_at(parent, path)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        SelectorSource::None => String::new(),
    };

    let mut spec = Map::new();
    let spec_replicas = replicas_at(parent, &scale.spec_replicas);
    if spec_replicas != 0 {
        spec.insert("replicas".to_string(), json!(spec_replicas));
    }
    let mut status = Map::new();
    status.insert(
        "replicas".to_string(),
        json!(replicas_at(parent, &scale.status_replicas)),
    );
    if !selector.is_empty() {
        status.insert("selector".to_string(), json!(selector));
    }

    Ok(json!({
        "apiVersion": "autoscaling/v1",
        "kind": "Scale",
        "metadata": metadata,
        "spec": spec,
        "status": status,
    }))
}

/// Writes `replicas` at the parent's spec replicas path.
pub(crate) fn set_spec_replicas(parent: &mut Value, scale: &ScaleSubresource, replicas: i64) {
    let (last, parents) = scale
        .spec_replicas
        .split_last()
        .expect("scale replica paths are never empty");
    let mut target = parent;
    for key in parents {
        target = &mut target[key.as_str()];
    }
    target[last.as_str()] = json!(replicas);
}

fn value_at<'a>(object: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(object, |value, key| value.get(key))
}

fn replicas_at(object: &Value, path: &[String]) -> i64 {
    value_at(object, path).and_then(Value::as_i64).unwrap_or(0)
}

/// Formats a `LabelSelector` as `labels.Selector.String()` does: requirements
/// sorted by key and joined with `,`, values sorted inside `in` and `notin`.
/// A missing or empty selector gives an empty string.
pub(crate) fn selector_string(selector: &Value) -> Result<String> {
    let mut requirements: Vec<(String, String)> = Vec::new();

    if let Some(labels) = selector.get("matchLabels").and_then(Value::as_object) {
        for (key, value) in labels {
            let value = value.as_str().unwrap_or_default();
            requirements.push((key.clone(), format!("{key}={value}")));
        }
    }

    for expr in selector
        .get("matchExpressions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = expr["key"].as_str().unwrap_or_default();
        let mut values: Vec<&str> = expr["values"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        values.sort_unstable();
        let requirement = match expr["operator"].as_str().unwrap_or_default() {
            "In" => format!("{key} in ({})", values.join(",")),
            "NotIn" => format!("{key} notin ({})", values.join(",")),
            "Exists" => key.to_string(),
            "DoesNotExist" => format!("!{key}"),
            operator => {
                return Err(Error::Internal(format!(
                    "{operator:?} is not a valid label selector operator"
                )))
            }
        };
        requirements.push((key.to_string(), requirement));
    }

    requirements.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(requirements
        .into_iter()
        .map(|(_, requirement)| requirement)
        .collect::<Vec<_>>()
        .join(","))
}

/// Formats a plain label map as `labels.SelectorFromSet(...).String()` does.
fn map_selector_string(selector: &Value) -> String {
    let mut pairs: Vec<String> = selector
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, value)| format!("{key}={}", value.as_str().unwrap_or_default()))
        .collect();
    pairs.sort_unstable();
    pairs.join(",")
}
