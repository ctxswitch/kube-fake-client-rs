//! Tests for the scale subresource conversion

use super::scale::{selector_string, to_scale, ScaleSubresource};
use serde_json::json;

#[test]
fn test_selector_string_sorts_requirements_and_values() {
    let selector = json!({
        "matchLabels": { "tier": "web", "app": "shop" },
        "matchExpressions": [
            { "key": "env", "operator": "In", "values": ["prod", "dev"] },
            { "key": "zone", "operator": "NotIn", "values": ["b", "a"] },
            { "key": "canary", "operator": "DoesNotExist" },
            { "key": "owner", "operator": "Exists" }
        ]
    });

    assert_eq!(
        selector_string(&selector).unwrap(),
        "app=shop,!canary,env in (dev,prod),owner,tier=web,zone notin (a,b)"
    );
}

#[test]
fn test_selector_string_empty_and_missing() {
    assert_eq!(selector_string(&json!({})).unwrap(), "");
    assert_eq!(selector_string(&json!(null)).unwrap(), "");
}

#[test]
fn test_selector_string_rejects_unknown_operator() {
    let selector = json!({
        "matchExpressions": [{ "key": "app", "operator": "Near", "values": ["x"] }]
    });
    assert!(selector_string(&selector).is_err());
}

#[test]
fn test_to_scale_replication_controller_map_selector() {
    let rc = json!({
        "apiVersion": "v1",
        "kind": "ReplicationController",
        "metadata": { "name": "rc", "namespace": "default", "uid": "u1", "resourceVersion": "7" },
        "spec": { "replicas": 2, "selector": { "tier": "web", "app": "shop" } },
        "status": { "replicas": 1 }
    });

    let scale = to_scale(&rc, &ScaleSubresource::built_in("ReplicationController")).unwrap();

    assert_eq!(
        scale,
        json!({
            "apiVersion": "autoscaling/v1",
            "kind": "Scale",
            "metadata": { "name": "rc", "namespace": "default", "uid": "u1", "resourceVersion": "7" },
            "spec": { "replicas": 2 },
            "status": { "replicas": 1, "selector": "app=shop,tier=web" }
        })
    );
}
