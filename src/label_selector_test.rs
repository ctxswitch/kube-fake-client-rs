//! Tests for label selector parsing and matching

use super::label_selector::{matches_label_selector, parse_label_selector};
use std::collections::BTreeMap;

#[test]
fn test_empty_selector() {
    let labels = BTreeMap::from([("app".to_string(), "test".to_string())]);
    assert!(matches_label_selector(&labels, "").unwrap());
    assert!(matches_label_selector(&labels, "   ").unwrap());
}

#[test]
fn test_equality_selector() {
    let labels = BTreeMap::from([
        ("app".to_string(), "myapp".to_string()),
        ("env".to_string(), "production".to_string()),
    ]);

    assert!(matches_label_selector(&labels, "app=myapp").unwrap());
    assert!(matches_label_selector(&labels, "app==myapp").unwrap());
    assert!(!matches_label_selector(&labels, "app=other").unwrap());
}

#[test]
fn test_inequality_selector() {
    let labels = BTreeMap::from([("env".to_string(), "production".to_string())]);

    assert!(matches_label_selector(&labels, "env!=staging").unwrap());
    assert!(!matches_label_selector(&labels, "env!=production").unwrap());
}

#[test]
fn test_in_selector() {
    let labels = BTreeMap::from([("env".to_string(), "production".to_string())]);

    assert!(matches_label_selector(&labels, "env in (production,staging)").unwrap());
    assert!(!matches_label_selector(&labels, "env in (development,testing)").unwrap());
}

#[test]
fn test_notin_selector() {
    let labels = BTreeMap::from([("env".to_string(), "production".to_string())]);

    assert!(matches_label_selector(&labels, "env notin (development,testing)").unwrap());
    assert!(!matches_label_selector(&labels, "env notin (production,staging)").unwrap());
}

#[test]
fn test_exists_selector() {
    let labels = BTreeMap::from([("app".to_string(), "myapp".to_string())]);

    assert!(matches_label_selector(&labels, "app").unwrap());
    assert!(!matches_label_selector(&labels, "env").unwrap());
}

#[test]
fn test_not_exists_selector() {
    let labels = BTreeMap::from([("app".to_string(), "myapp".to_string())]);

    assert!(matches_label_selector(&labels, "!env").unwrap());
    assert!(!matches_label_selector(&labels, "!app").unwrap());
}

#[test]
fn test_combined_selectors() {
    let labels = BTreeMap::from([
        ("app".to_string(), "myapp".to_string()),
        ("env".to_string(), "production".to_string()),
    ]);

    assert!(matches_label_selector(&labels, "app=myapp,env=production").unwrap());
    assert!(matches_label_selector(&labels, "app=myapp,env in (production,staging)").unwrap());
    assert!(!matches_label_selector(&labels, "app=myapp,env=staging").unwrap());
}

#[test]
fn test_complex_combined_selectors() {
    let labels = BTreeMap::from([
        ("app".to_string(), "myapp".to_string()),
        ("tier".to_string(), "frontend".to_string()),
    ]);

    // Combine existence and equality
    assert!(matches_label_selector(&labels, "app,tier=frontend").unwrap());

    // Combine inequality and existence
    assert!(matches_label_selector(&labels, "app,tier!=backend").unwrap());

    // Combine notin and equality
    assert!(matches_label_selector(&labels, "app=myapp,tier notin (backend,middleware)").unwrap());
}

#[test]
fn test_no_labels() {
    let labels = BTreeMap::new();

    // Empty labels should match !key selectors
    assert!(matches_label_selector(&labels, "!app").unwrap());

    // Empty labels should not match existence selectors
    assert!(!matches_label_selector(&labels, "app").unwrap());

    // Empty labels should not match equality selectors
    assert!(!matches_label_selector(&labels, "app=myapp").unwrap());
}

#[test]
fn test_parse_label_selector_valid() {
    // Test that parsing doesn't fail for valid selectors
    assert!(parse_label_selector("app=myapp").is_ok());
    assert!(parse_label_selector("env in (prod,staging)").is_ok());
    assert!(parse_label_selector("!debug").is_ok());
    assert!(parse_label_selector("app=myapp,tier=frontend").is_ok());
}

#[test]
fn test_parse_label_selector_invalid() {
    // Test invalid 'in' syntax
    assert!(parse_label_selector("env in prod").is_err());
    assert!(parse_label_selector("env in (prod").is_err());

    // Test invalid 'notin' syntax
    assert!(parse_label_selector("env notin prod").is_err());
    assert!(parse_label_selector("env notin prod)").is_err());
}
