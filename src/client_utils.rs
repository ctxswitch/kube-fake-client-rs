use crate::{tracker::GVK, Error, Result};
use serde_json::Value;

/// Pluralize a Kubernetes Kind name to its resource plural form.
///
/// Implementation copied from kube-rs to ensure consistency with the broader ecosystem.
/// See: <https://github.com/kube-rs/kube/blob/main/kube-core/src/discovery.rs>
///
/// Copyright (c) kube-rs contributors
/// Licensed under Apache-2.0
///
/// This follows the same pluralization rules:
/// - Special cases for already-plural words (endpoints, endpointslices)
/// - Special cases for metrics resources (nodemetrics, podmetrics)
/// - Words ending in s, x, z, ch, sh get -es suffix
/// - Words ending in consonant+y get -ies suffix
/// - All other words get -s suffix
pub fn pluralize(kind: &str) -> String {
    let word = kind.to_ascii_lowercase();

    // Special cases for already-plural or irregular resources
    if word == "endpoints" || word == "endpointslices" {
        return word;
    } else if word == "nodemetrics" {
        return "nodes".to_string();
    } else if word == "podmetrics" {
        return "pods".to_string();
    }

    // Words ending in s, x, z, ch, sh will be pluralized with -es (eg. foxes).
    if word.ends_with('s')
        || word.ends_with('x')
        || word.ends_with('z')
        || word.ends_with("ch")
        || word.ends_with("sh")
    {
        return format!("{word}es");
    }

    // Words ending in y that are preceded by a consonant will be pluralized by
    // replacing y with -ies (eg. puppies).
    if word.ends_with('y') {
        if let Some(c) = word.chars().nth(word.len() - 2) {
            if !matches!(c, 'a' | 'e' | 'i' | 'o' | 'u') {
                // Remove 'y' and add `ies`
                let mut chars = word.chars();
                chars.next_back();
                return format!("{}ies", chars.as_str());
            }
        }
    }

    // All other words will have "s" added to the end (eg. days).
    format!("{word}s")
}

pub fn extract_gvk(value: &Value) -> Result<GVK> {
    let api_version = value
        .get("apiVersion")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidRequest("Missing apiVersion".to_string()))?;

    let kind = value
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidRequest("Missing kind".to_string()))?;

    let (group, version) = if let Some((g, v)) = api_version.split_once('/') {
        (g.to_string(), v.to_string())
    } else {
        ("".to_string(), api_version.to_string())
    };

    Ok(GVK::new(group, version, kind))
}
