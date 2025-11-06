use crate::{tracker::GVK, Error, Result};
use serde_json::Value;

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
