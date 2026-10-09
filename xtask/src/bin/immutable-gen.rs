//! Immutable field lookup generator for kube-fake-client
//!
//! This binary generates Rust code for looking up immutable fields in Kubernetes resources.
//! Immutable fields are fields that cannot be changed after resource creation.
//!
//! The generator parses the Kubernetes OpenAPI schema (swagger.json) and identifies fields
//! whose descriptions contain the word "immutable".
//!
//! The generated lookups cover every release in `xtask::RELEASES`: a field is immutable
//! if it is immutable in any of those releases. The generator prints a warning when a
//! field is immutable in one release and mutable in another.
//!
//! Each release reads `kubernetes/api/openapi/<minor>/swagger.json` and fetches the
//! file from the Kubernetes GitHub repo when it is missing.
//!
//! # Usage
//!
//! Generate immutable field lookups from local swagger.json files:
//! ```bash
//! cargo run -p xtask --bin immutable-gen
//! ```
//!
//! Fetch swagger.json for every release again, then generate:
//! ```bash
//! cargo run -p xtask --bin immutable-gen -- --update
//! ```

use clap::Parser;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tera::{Context, Tera};
use xtask::{fetch_file, release_dir, RELEASES};

// Directory and file names for Kubernetes OpenAPI schema
const OPENAPI_DIR: &str = "kubernetes/api/openapi";
const OPENAPI_FILE: &str = "swagger.json";

const USER_AGENT: &str = "kube-fake-client-immutable-gen";

/// Definition key: (group, version, kind)
type DefinitionKey = (String, String, String);

/// Fields of one OpenAPI definition in one release
#[derive(Default)]
struct DefinitionFields {
    all: BTreeSet<String>,
    immutable: BTreeSet<String>,
}

#[derive(Parser, Debug)]
#[command(name = "immutable-gen")]
#[command(about = "Generate immutable field lookups from OpenAPI schema", long_about = None)]
struct Args {
    /// Fetch the OpenAPI schema for every release from the Kubernetes GitHub repository
    #[arg(short, long)]
    update: bool,

    /// Output directory for generated code (default: src/gen)
    #[arg(short, long, default_value = "src/gen")]
    output: PathBuf,
}

/// Immutable field information for a resource type
#[derive(Debug, Serialize)]
struct ImmutableFieldInfo {
    group: String,       // e.g., "batch" or "" for core
    version: String,     // e.g., "v1"
    kind: String,        // e.g., "JobSpec" or "ObjectMeta"
    fields: Vec<String>, // e.g., ["nodeName", "serviceAccountName"]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    fs::create_dir_all(&args.output)?;

    let mut releases = Vec::new();
    for tag in RELEASES {
        let swagger_path = release_dir(OPENAPI_DIR, tag).join(OPENAPI_FILE);

        if args.update || !swagger_path.exists() {
            println!(
                "Fetching OpenAPI schema from Kubernetes GitHub repo (tag: {})...",
                tag
            );
            fetch_file(
                USER_AGENT,
                tag,
                "api/openapi-spec/swagger.json",
                &swagger_path,
            )?;
        }

        println!("Parsing {} for immutable fields...", swagger_path.display());
        releases.push((*tag, parse_definitions(&swagger_path)?));
    }

    let immutable_fields = merge_releases(&releases);
    println!(
        "Found {} definitions with immutable fields",
        immutable_fields.len()
    );

    // Generate immutable field lookup code
    println!("Generating immutable field lookups...");
    let output_path = args.output.join("immutable.rs");
    generate_immutable_code(&immutable_fields, &output_path)?;
    println!("Generated code written to {}", output_path.display());

    Ok(())
}

/// Parse OpenAPI definition name to extract (group, version, kind)
///
/// Examples:
/// - "io.k8s.api.batch.v1.JobSpec" -> ("batch", "v1", "JobSpec")
/// - "io.k8s.api.core.v1.PodSpec" -> ("", "v1", "PodSpec")  // core is empty group
/// - "io.k8s.apimachinery.pkg.apis.meta.v1.ObjectMeta" -> ("", "v1", "ObjectMeta")
fn parse_definition_name(def_name: &str) -> Result<DefinitionKey, String> {
    if let Some(rest) = def_name.strip_prefix("io.k8s.api.") {
        // Standard resource: io.k8s.api.{group}.{version}.{Kind}
        let parts: Vec<&str> = rest.split('.').collect();
        if parts.len() < 3 {
            return Err(format!("Invalid definition name: {}", def_name));
        }

        // Check if this is a core resource (io.k8s.api.core.v1.Kind)
        if parts[0] == "core" {
            // Core resources have empty group
            Ok(("".to_string(), parts[1].to_string(), parts[2].to_string()))
        } else {
            // Non-core: group is first part
            Ok((
                parts[0].to_string(),
                parts[1].to_string(),
                parts[2].to_string(),
            ))
        }
    } else if let Some(rest) = def_name.strip_prefix("io.k8s.apimachinery.pkg.apis.meta.") {
        // apimachinery types: io.k8s.apimachinery.pkg.apis.meta.{version}.{Kind}
        // Treat these as core (empty group) since they're fundamental types
        let parts: Vec<&str> = rest.split('.').collect();
        if parts.len() < 2 {
            return Err(format!(
                "Invalid apimachinery definition name: {}",
                def_name
            ));
        }
        Ok(("".to_string(), parts[0].to_string(), parts[1].to_string()))
    } else {
        Err(format!("Unknown definition name format: {}", def_name))
    }
}

/// Parse one swagger.json into the fields of each definition that has immutable fields
fn parse_definitions(
    path: &Path,
) -> Result<BTreeMap<DefinitionKey, DefinitionFields>, Box<dyn std::error::Error>> {
    use serde_json::Value;

    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let swagger: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

    let definitions = swagger
        .get("definitions")
        .and_then(|d| d.as_object())
        .ok_or("OpenAPI spec missing 'definitions'")?;

    let mut parsed = BTreeMap::new();

    for (def_name, def_obj) in definitions {
        let Some(properties) = def_obj.get("properties").and_then(|p| p.as_object()) else {
            continue;
        };

        let mut fields = DefinitionFields::default();
        for (field_name, field_obj) in properties {
            fields.all.insert(field_name.clone());

            // Skip fields named "immutable" - these are control flags, not immutable fields
            if field_name == "immutable" {
                continue;
            }

            if let Some(description) = field_obj.get("description").and_then(|d| d.as_str()) {
                // Check if the description mentions "immutable" (case-insensitive)
                if description.to_lowercase().contains("immutable") {
                    fields.immutable.insert(field_name.clone());
                }
            }
        }

        match parse_definition_name(def_name) {
            Ok(key) => {
                parsed.insert(key, fields);
            }
            Err(e) if !fields.immutable.is_empty() => {
                eprintln!("Warning: Skipping definition '{}': {}", def_name, e);
            }
            Err(_) => {}
        }
    }

    Ok(parsed)
}

/// Merge the immutable fields of every release, sorted by (group, version, kind)
fn merge_releases(
    releases: &[(&str, BTreeMap<DefinitionKey, DefinitionFields>)],
) -> Vec<ImmutableFieldInfo> {
    let mut merged: BTreeMap<DefinitionKey, BTreeSet<String>> = BTreeMap::new();

    for (_, definitions) in releases {
        for (key, fields) in definitions {
            if !fields.immutable.is_empty() {
                merged
                    .entry(key.clone())
                    .or_default()
                    .extend(fields.immutable.iter().cloned());
            }
        }
    }

    // A field that a release serves without "immutable" in its description
    // is mutable in that release.
    for (key, immutable) in &merged {
        for (tag, definitions) in releases {
            let Some(fields) = definitions.get(key) else {
                continue;
            };
            for field in immutable {
                if fields.all.contains(field) && !fields.immutable.contains(field) {
                    eprintln!(
                        "Warning: {}/{}/{} field '{}' is mutable in {} but immutable in another release",
                        key.0, key.1, key.2, field, tag
                    );
                }
            }
        }
    }

    // Add common immutable fields from ObjectMeta
    // These are immutable after creation but may not have "immutable" in their descriptions
    merged
        .entry(("".to_string(), "v1".to_string(), "ObjectMeta".to_string()))
        .or_default()
        .extend(
            [
                "creationTimestamp",
                "generateName",
                "generation",
                "name",
                "namespace",
                "uid",
            ]
            .map(String::from),
        );

    merged
        .into_iter()
        .map(|((group, version, kind), fields)| ImmutableFieldInfo {
            group,
            version,
            kind,
            fields: fields.into_iter().collect(),
        })
        .collect()
}

/// Template for generating immutable.rs
const IMMUTABLE_TEMPLATE: &str = r#"//! Auto-generated immutable field lookups
//!
//! This file is generated by the immutable-gen binary and should not be edited manually.
//! To regenerate: cargo run -p xtask --bin immutable-gen
//!
//! Immutable fields are fields that cannot be changed after resource creation.
//! This module provides lookups to check if a field in a Kubernetes resource is immutable.

/// Check if a specific field in a resource type is immutable
///
/// # Arguments
///
/// * `group` - The API group (empty string for core resources)
/// * `version` - The API version (e.g., "v1")
/// * `kind` - The kind/type name (e.g., "PodSpec", "ObjectMeta")
/// * `field_name` - The field name to check (e.g., "name", "resourceClaims")
///
/// # Returns
///
/// `true` if the field is immutable, `false` otherwise
///
/// # Notes
///
/// This function also recognizes TypeMeta fields (`apiVersion` and `kind`) as immutable
/// for all resource types, even though they are inlined on each resource rather than
/// being in a separate TypeMeta definition.
///
/// # Example
///
/// ```
/// use kube_fake_client::gen::immutable::is_field_immutable;
///
/// // Resource-specific immutable fields
/// assert!(is_field_immutable("", "v1", "PodSpec", "resourceClaims"));
/// assert!(!is_field_immutable("", "v1", "PodSpec", "containers"));
///
/// // ObjectMeta immutable fields
/// assert!(is_field_immutable("", "v1", "ObjectMeta", "name"));
/// assert!(is_field_immutable("", "v1", "ObjectMeta", "uid"));
///
/// // TypeMeta fields (recognized for any resource type)
/// assert!(is_field_immutable("", "v1", "Pod", "apiVersion"));
/// assert!(is_field_immutable("", "v1", "Pod", "kind"));
/// assert!(is_field_immutable("apps", "v1", "Deployment", "apiVersion"));
/// ```
pub fn is_field_immutable(group: &str, version: &str, kind: &str, field_name: &str) -> bool {
    // TypeMeta fields are always immutable (inlined on all Kubernetes resources)
    if field_name == "apiVersion" || field_name == "kind" {
        return true;
    }

    if let Some(fields) = get_immutable_fields(group, version, kind) {
        fields.contains(&field_name)
    } else {
        false
    }
}

/// Get all immutable fields for a given resource type
///
/// # Arguments
///
/// * `group` - The API group (empty string for core resources)
/// * `version` - The API version (e.g., "v1")
/// * `kind` - The kind/type name (e.g., "PodSpec", "ObjectMeta")
///
/// # Returns
///
/// `Some(&[&str])` containing the immutable field names if any exist, `None` otherwise
///
/// # Example
///
/// ```
/// use kube_fake_client::gen::immutable::get_immutable_fields;
///
/// if let Some(fields) = get_immutable_fields("", "v1", "PodSpec") {
///     for field in fields {
///         println!("Immutable field: {}", field);
///     }
/// }
/// ```
pub fn get_immutable_fields(group: &str, version: &str, kind: &str) -> Option<&'static [&'static str]> {
    match (group, version, kind) {
{% for info in immutable_fields %}        ("{{ info.group }}", "{{ info.version }}", "{{ info.kind }}") => Some(&[{% for field in info.fields %}"{{ field }}"{% if not loop.last %}, {% endif %}{% endfor %}]),
{% endfor %}        _ => None,
    }
}
"#;

/// Generate immutable field lookup code
fn generate_immutable_code(
    immutable_fields: &[ImmutableFieldInfo],
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut tera = Tera::default();
    tera.add_raw_template("immutable", IMMUTABLE_TEMPLATE)?;

    let mut context = Context::new();
    context.insert("immutable_fields", immutable_fields);

    let rendered = tera.render("immutable", &context)?;
    fs::write(output_path, rendered)?;

    Ok(())
}
