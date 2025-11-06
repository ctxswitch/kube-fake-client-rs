//! Immutable field lookup generator for kube-fake-client
//!
//! This binary generates Rust code for looking up immutable fields in Kubernetes resources.
//! Immutable fields are fields that cannot be changed after resource creation.
//!
//! The generator parses the Kubernetes OpenAPI schema (swagger.json) and identifies fields
//! whose descriptions contain the word "immutable".
//!
//! # Usage
//!
//! Generate immutable field lookups from local swagger.json:
//! ```bash
//! cargo run --bin immutable-gen
//! ```
//!
//! Update swagger.json from Kubernetes GitHub repo:
//! ```bash
//! cargo run --bin immutable-gen -- --update
//! ```
//!
//! Target a specific Kubernetes version:
//! ```bash
//! cargo run --bin immutable-gen -- --update --tag v1.31.0
//! ```

use clap::Parser;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use tera::{Context, Tera};

// Directory and file paths for Kubernetes OpenAPI schema
const OPENAPI_DIR: &str = "kubernetes/api/openapi";
const OPENAPI_FILE: &str = "kubernetes/api/openapi/swagger.json";

// GitHub repository configuration
const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com/kubernetes/kubernetes";
const USER_AGENT: &str = "kube-fake-client-immutable-gen";

#[derive(Parser, Debug)]
#[command(name = "immutable-gen")]
#[command(about = "Generate immutable field lookups from OpenAPI schema", long_about = None)]
struct Args {
    /// Update OpenAPI schema from Kubernetes GitHub repository
    #[arg(short, long)]
    update: bool,

    /// Git tag or SHA to fetch from (default: master)
    #[arg(short, long, default_value = "master")]
    tag: String,

    /// Output directory for generated code (default: src/gen)
    #[arg(short, long, default_value = "src/gen")]
    output: PathBuf,
}

/// Immutable field information for a resource type
#[derive(Debug, Serialize)]
struct ImmutableFieldInfo {
    group: String,      // e.g., "batch" or "" for core
    version: String,    // e.g., "v1"
    kind: String,       // e.g., "JobSpec" or "ObjectMeta"
    fields: Vec<String>, // e.g., ["nodeName", "serviceAccountName"]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Ensure directories exist
    fs::create_dir_all(OPENAPI_DIR)?;
    fs::create_dir_all(&args.output)?;

    // Check if swagger.json exists
    let swagger_path = Path::new(OPENAPI_FILE);
    let file_exists = swagger_path.exists();

    // Fetch file if --update is specified or file doesn't exist
    if args.update || !file_exists {
        println!(
            "Fetching OpenAPI schema from Kubernetes GitHub repo (tag: {})...",
            args.tag
        );
        fetch_openapi_file(&args.tag)?;
        println!("OpenAPI schema updated successfully");
    }

    // Parse OpenAPI schema for immutable fields
    println!("Parsing OpenAPI schema for immutable fields...");
    let immutable_fields = parse_immutable_fields()?;
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

/// Create an HTTP client for fetching files from GitHub
fn create_http_client() -> Result<reqwest::blocking::Client, Box<dyn std::error::Error>> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e).into())
}

/// Fetch a file from GitHub and save it to disk
fn fetch_file(
    client: &reqwest::blocking::Client,
    url: &str,
    save_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Fetching {}...", url);
    let response = client.get(url).send()?;

    if !response.status().is_success() {
        return Err(format!("Failed to fetch {}: HTTP {}", url, response.status()).into());
    }

    let content = response.text()?;
    fs::write(save_path, content)?;
    Ok(())
}

/// Fetch OpenAPI swagger file from Kubernetes GitHub repository
fn fetch_openapi_file(tag: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = create_http_client()?;

    let swagger_url = format!("{}/{}/api/openapi-spec/swagger.json", GITHUB_RAW_BASE, tag);
    fetch_file(&client, &swagger_url, OPENAPI_FILE)?;

    Ok(())
}

/// Parse OpenAPI definition name to extract (group, version, kind)
///
/// Examples:
/// - "io.k8s.api.batch.v1.JobSpec" -> ("batch", "v1", "JobSpec")
/// - "io.k8s.api.core.v1.PodSpec" -> ("", "v1", "PodSpec")  // core is empty group
/// - "io.k8s.apimachinery.pkg.apis.meta.v1.ObjectMeta" -> ("", "v1", "ObjectMeta")
fn parse_definition_name(def_name: &str) -> Result<(String, String, String), String> {
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
            Ok((parts[0].to_string(), parts[1].to_string(), parts[2].to_string()))
        }
    } else if let Some(rest) = def_name.strip_prefix("io.k8s.apimachinery.pkg.apis.meta.") {
        // apimachinery types: io.k8s.apimachinery.pkg.apis.meta.{version}.{Kind}
        // Treat these as core (empty group) since they're fundamental types
        let parts: Vec<&str> = rest.split('.').collect();
        if parts.len() < 2 {
            return Err(format!("Invalid apimachinery definition name: {}", def_name));
        }
        Ok(("".to_string(), parts[0].to_string(), parts[1].to_string()))
    } else {
        Err(format!("Unknown definition name format: {}", def_name))
    }
}

/// Parse OpenAPI swagger.json to find immutable fields
fn parse_immutable_fields() -> Result<Vec<ImmutableFieldInfo>, Box<dyn std::error::Error>> {
    use serde_json::Value;

    // Load swagger.json
    let content = fs::read_to_string(OPENAPI_FILE)
        .map_err(|e| format!("Failed to read {}: {}", OPENAPI_FILE, e))?;

    let swagger: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", OPENAPI_FILE, e))?;

    let definitions = swagger
        .get("definitions")
        .and_then(|d| d.as_object())
        .ok_or("OpenAPI spec missing 'definitions'")?;

    let mut immutable_info = Vec::new();

    // Add common immutable fields from ObjectMeta
    // These are immutable after creation but may not have "immutable" in their descriptions
    immutable_info.push(ImmutableFieldInfo {
        group: "".to_string(),
        version: "v1".to_string(),
        kind: "ObjectMeta".to_string(),
        fields: vec![
            "creationTimestamp".to_string(),
            "generateName".to_string(),
            "generation".to_string(),
            "name".to_string(),
            "namespace".to_string(),
            "uid".to_string(),
        ],
    });

    // Scan each definition for immutable fields
    for (def_name, def_obj) in definitions {
        if let Some(properties) = def_obj.get("properties").and_then(|p| p.as_object()) {
            let mut immutable_fields = Vec::new();

            for (field_name, field_obj) in properties {
                // Skip fields named "immutable" - these are control flags, not immutable fields
                if field_name == "immutable" {
                    continue;
                }

                if let Some(description) = field_obj.get("description").and_then(|d| d.as_str()) {
                    // Check if the description mentions "immutable" (case-insensitive)
                    if description.to_lowercase().contains("immutable") {
                        immutable_fields.push(field_name.clone());
                    }
                }
            }

            // Only include definitions that have immutable fields
            if !immutable_fields.is_empty() {
                // Parse the definition name to extract group, version, kind
                match parse_definition_name(def_name) {
                    Ok((group, version, kind)) => {
                        immutable_info.push(ImmutableFieldInfo {
                            group,
                            version,
                            kind,
                            fields: immutable_fields,
                        });
                    }
                    Err(e) => {
                        eprintln!("Warning: Skipping definition '{}': {}", def_name, e);
                    }
                }
            }
        }
    }

    // Sort by (group, version, kind) for consistent output
    immutable_info.sort_by(|a, b| {
        a.group
            .cmp(&b.group)
            .then(a.version.cmp(&b.version))
            .then(a.kind.cmp(&b.kind))
    });

    Ok(immutable_info)
}

/// Template for generating immutable.rs
const IMMUTABLE_TEMPLATE: &str = r#"//! Auto-generated immutable field lookups
//!
//! This file is generated by the immutable-gen binary and should not be edited manually.
//! To regenerate: cargo run --bin immutable-gen
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
