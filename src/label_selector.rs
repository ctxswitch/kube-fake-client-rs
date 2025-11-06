//! Label selector parsing and matching for filtering Kubernetes resources
//!
//! This module provides comprehensive support for parsing Kubernetes label selector strings
//! and matching them against resource labels using `kube::core::Selector`.
//!
//! Supports all Kubernetes label selector syntax:
//! - Equality: `key=value` or `key==value`
//! - Inequality: `key!=value`
//! - Set-based: `key in (value1,value2)` or `key notin (value1,value2)`
//! - Existence: `key` or `!key`
//! - Multiple selectors combined with commas: `key1=value1,key2 in (v2,v3)`

use kube::core::{Expression, Selector, SelectorExt};
use std::collections::{BTreeMap, BTreeSet};

/// Split a selector string by commas, but not inside parentheses
fn split_preserving_parentheses(selector: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0;

    for (i, ch) in selector.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                result.push(&selector[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }

    // Add the last segment
    if start < selector.len() {
        result.push(&selector[start..]);
    }

    result
}

/// Parse a Kubernetes label selector string into a Selector
///
/// Returns `Ok(Selector)` if parsing succeeds, or `Err(String)` with error message if parsing fails.
///
/// # Examples
///
/// ```
/// use kube_fake_client::label_selector::parse_label_selector;
///
/// // Equality
/// let selector = parse_label_selector("app=myapp").unwrap();
///
/// // Set-based
/// let selector = parse_label_selector("env in (production,staging)").unwrap();
///
/// // Combined
/// let selector = parse_label_selector("app=myapp,env in (production,staging)").unwrap();
/// ```
pub fn parse_label_selector(selector: &str) -> Result<Selector, String> {
    if selector.trim().is_empty() {
        // Empty selector matches everything
        return Ok(Selector::default());
    }

    let mut expressions = Vec::new();

    // Split by comma, but not inside parentheses
    let requirements = split_preserving_parentheses(selector);

    for requirement in requirements {
        let requirement = requirement.trim();
        if requirement.is_empty() {
            continue;
        }

        // Check for set-based operators: "in" and "notin"
        if let Some((key, rest)) = requirement.split_once(" in ") {
            let key = key.trim();
            // Parse values: (value1,value2,value3)
            if !rest.starts_with('(') || !rest.ends_with(')') {
                return Err(format!("Invalid 'in' syntax: {}", requirement));
            }
            let values: BTreeSet<String> = rest[1..rest.len() - 1]
                .split(',')
                .map(|v| v.trim().to_string())
                .collect();
            expressions.push(Expression::In(key.to_string(), values));
        } else if let Some((key, rest)) = requirement.split_once(" notin ") {
            let key = key.trim();
            if !rest.starts_with('(') || !rest.ends_with(')') {
                return Err(format!("Invalid 'notin' syntax: {}", requirement));
            }
            let values: BTreeSet<String> = rest[1..rest.len() - 1]
                .split(',')
                .map(|v| v.trim().to_string())
                .collect();
            expressions.push(Expression::NotIn(key.to_string(), values));
        } else if let Some(key) = requirement.strip_prefix('!') {
            // Existence operator: !key
            let key = key.trim();
            expressions.push(Expression::DoesNotExist(key.to_string()));
        } else if requirement.contains("!=") {
            // Inequality operator
            let parts: Vec<&str> = requirement.splitn(2, "!=").collect();
            if parts.len() != 2 {
                return Err(format!("Invalid inequality syntax: {}", requirement));
            }
            let key = parts[0].trim();
            let value = parts[1].trim();
            // NotIn with single value is equivalent to !=
            let mut values = BTreeSet::new();
            values.insert(value.to_string());
            expressions.push(Expression::NotIn(key.to_string(), values));
        } else if requirement.contains("==") {
            // Equality operator (==)
            let parts: Vec<&str> = requirement.splitn(2, "==").collect();
            if parts.len() != 2 {
                return Err(format!("Invalid equality syntax: {}", requirement));
            }
            let key = parts[0].trim();
            let value = parts[1].trim();
            let mut values = BTreeSet::new();
            values.insert(value.to_string());
            expressions.push(Expression::In(key.to_string(), values));
        } else if requirement.contains('=') {
            // Equality operator (=)
            let parts: Vec<&str> = requirement.splitn(2, '=').collect();
            if parts.len() != 2 {
                return Err(format!("Invalid equality syntax: {}", requirement));
            }
            let key = parts[0].trim();
            let value = parts[1].trim();
            let mut values = BTreeSet::new();
            values.insert(value.to_string());
            expressions.push(Expression::In(key.to_string(), values));
        } else {
            // Existence operator: key (no operator)
            expressions.push(Expression::Exists(requirement.to_string()));
        }
    }

    // Combine all expressions into a single selector (AND semantics)
    if expressions.is_empty() {
        Ok(Selector::default())
    } else {
        // Convert Vec<Expression> to Selector
        // Multiple expressions are combined with AND semantics
        Ok(Selector::from_iter(expressions))
    }
}

/// Match labels against a label selector string
///
/// Returns `Ok(true)` if the labels match the selector, `Ok(false)` if they don't match,
/// or `Err(String)` if the selector string is invalid.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
/// use kube_fake_client::label_selector::matches_label_selector;
///
/// let labels = BTreeMap::from([
///     ("app".to_string(), "myapp".to_string()),
///     ("env".to_string(), "production".to_string()),
/// ]);
///
/// assert!(matches_label_selector(&labels, "app=myapp").unwrap());
/// assert!(matches_label_selector(&labels, "env in (production,staging)").unwrap());
/// assert!(!matches_label_selector(&labels, "app=other").unwrap());
/// ```
pub fn matches_label_selector(
    labels: &BTreeMap<String, String>,
    selector: &str,
) -> Result<bool, String> {
    let selector = parse_label_selector(selector)?;
    Ok(selector.matches(labels))
}
