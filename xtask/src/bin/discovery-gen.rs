//! Discovery metadata generator for kube-fake-client
//!
//! This binary generates Rust code from Kubernetes API discovery metadata.
//! Discovery data includes resource metadata, GVK mappings, verbs, and subresources.
//!
//! The generated lookups cover every release in `xtask::RELEASES`. A resource
//! present in any release is known; its verbs, subresources, and short names are
//! the union over those releases, and its other metadata comes from the newest
//! release that serves it.
//!
//! The data of each release is in `kubernetes/api/discovery/<minor>/`. The
//! generator fetches missing files from the Kubernetes GitHub repo.
//!
//! # Usage
//!
//! Generate discovery code from local files:
//! ```bash
//! cargo run -p xtask --bin discovery-gen
//! ```
//!
//! Fetch discovery data for every release, then generate:
//! ```bash
//! cargo run -p xtask --bin discovery-gen -- --update
//! ```

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tera::{Context, Tera};
use xtask::{fetch_file, release_dir, RELEASES};

// Directory paths for Kubernetes API files
const DISCOVERY_DIR: &str = "kubernetes/api/discovery";
const DISCOVERY_FILES: [&str; 2] = ["api__v1.json", "aggregated_v2.json"];

const USER_AGENT: &str = "kube-fake-client-discovery-gen";

#[derive(Parser, Debug)]
#[command(name = "discovery-gen")]
#[command(about = "Generate Kubernetes discovery code from JSON files", long_about = None)]
struct Args {
    /// Fetch discovery data for every release from the Kubernetes GitHub repository
    #[arg(short, long)]
    update: bool,

    /// Output directory for generated code (default: src/gen)
    #[arg(short, long, default_value = "src/gen")]
    output: PathBuf,
}

// ============================================================================
// Discovery API Data Structures (from aggregated_v2.json)
// ============================================================================

/// Top-level aggregated discovery document containing all API groups
#[derive(Debug, Deserialize)]
struct AggregatedDiscovery {
    items: Vec<APIGroupDiscovery>,
}

/// Discovery information for a single API group (e.g., "apps", "batch")
#[derive(Debug, Deserialize)]
struct APIGroupDiscovery {
    metadata: Metadata,
    versions: Vec<APIVersionDiscovery>,
}

/// Metadata containing the API group name
#[derive(Debug, Deserialize)]
struct Metadata {
    name: String,
}

/// Discovery information for a specific version within an API group
#[derive(Debug, Deserialize)]
struct APIVersionDiscovery {
    version: String,
    resources: Vec<APIResource>,
}

/// Information about a specific resource type (e.g., "deployments")
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct APIResource {
    resource: String,
    response_kind: ResponseKind,
    scope: String,
    singular_resource: Option<String>,
    verbs: Vec<String>,
    #[serde(default)]
    subresources: Vec<APISubresource>,
    #[serde(default)]
    short_names: Vec<String>,
}

/// The Kind name returned by the API for a resource
#[derive(Debug, Deserialize)]
struct ResponseKind {
    kind: String,
}

/// Information about a subresource (e.g., "status", "scale")
#[derive(Debug, Deserialize)]
struct APISubresource {
    subresource: String,
    verbs: Vec<String>,
}

// ============================================================================
// Core API Data Structures (from api__v1.json)
// ============================================================================

/// Core API resource list (v1 API group has a different format)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreAPIResourceList {
    resources: Vec<CoreAPIResource>,
}

/// Core API resource (pods, services, etc.)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreAPIResource {
    kind: String,
    name: String,
    namespaced: bool,
    #[serde(default)]
    singular_name: String,
    verbs: Vec<String>,
    #[serde(default)]
    short_names: Vec<String>,
}

// ============================================================================
// Output Data Structures (for code generation)
// ============================================================================

/// Complete metadata for a Kubernetes resource type
#[derive(Debug, Serialize)]
struct ResourceMetadata {
    group: String,
    version: String,
    kind: String,
    plural: String,
    singular: String,
    namespaced: bool,
    verbs: Vec<String>,
    subresources: Vec<Subresource>,
    short_names: Vec<String>,
}

/// Subresource information (status, scale, etc.)
#[derive(Debug, Clone, Serialize)]
struct Subresource {
    name: String,
    verbs: Vec<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    fs::create_dir_all(&args.output)?;

    for tag in RELEASES {
        let dir = release_dir(DISCOVERY_DIR, tag);
        let files_exist = DISCOVERY_FILES.iter().all(|file| dir.join(file).exists());

        if args.update || !files_exist {
            println!(
                "Fetching discovery data from Kubernetes GitHub repo (tag: {})...",
                tag
            );
            for file in DISCOVERY_FILES {
                fetch_file(
                    USER_AGENT,
                    tag,
                    &format!("api/discovery/{}", file),
                    &dir.join(file),
                )?;
            }
        }
    }

    // Parse and generate discovery code
    println!("Parsing discovery data...");
    let resources = merge_releases()?;
    println!("Merged {} resources", resources.len());

    println!("Generating discovery code...");
    let output_path = args.output.join("discovery.rs");
    generate_discovery_code(&resources, &output_path)?;
    println!("Generated code written to {}", output_path.display());

    Ok(())
}

/// Extract subresources from core API resource list
/// Subresources have "/" in their name (e.g., "pods/status")
fn extract_core_subresources(resources: &[CoreAPIResource]) -> HashMap<String, Vec<Subresource>> {
    let mut subresources: HashMap<String, Vec<Subresource>> = HashMap::new();

    for resource in resources {
        if let Some(slash_pos) = resource.name.find('/') {
            let parent_name = &resource.name[..slash_pos];
            let subresource_name = &resource.name[slash_pos + 1..];

            subresources
                .entry(parent_name.to_string())
                .or_default()
                .push(Subresource {
                    name: subresource_name.to_string(),
                    verbs: resource.verbs.clone(),
                });
        }
    }

    subresources
}

/// Parse the core API (v1) discovery file
fn parse_core_api(dir: &Path) -> Result<Vec<ResourceMetadata>, Box<dyn std::error::Error>> {
    let path = dir.join("api__v1.json");
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let api_list: CoreAPIResourceList = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

    // Extract subresources first
    let subresources_map = extract_core_subresources(&api_list.resources);

    // Build resource metadata for main resources only (not subresources)
    let mut resources = Vec::new();
    for resource in &api_list.resources {
        // Skip subresources (they have "/" in the name)
        if resource.name.contains('/') {
            continue;
        }

        let subresources = subresources_map
            .get(&resource.name)
            .cloned()
            .unwrap_or_default();

        resources.push(ResourceMetadata {
            group: String::new(), // Core API has empty group
            version: "v1".to_string(),
            kind: resource.kind.clone(),
            plural: resource.name.clone(),
            singular: if resource.singular_name.is_empty() {
                // Derive singular from plural if not provided
                resource.name.trim_end_matches('s').to_string()
            } else {
                resource.singular_name.clone()
            },
            namespaced: resource.namespaced,
            verbs: resource.verbs.clone(),
            subresources,
            short_names: resource.short_names.clone(),
        });
    }

    Ok(resources)
}

/// Parse the aggregated discovery file
fn parse_aggregated_discovery(
    dir: &Path,
) -> Result<Vec<ResourceMetadata>, Box<dyn std::error::Error>> {
    let path = dir.join("aggregated_v2.json");
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let discovery: AggregatedDiscovery = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

    let mut resources = Vec::new();

    for group in &discovery.items {
        for version in &group.versions {
            for resource in &version.resources {
                // Skip subresources (they have "/" in the resource name)
                if resource.resource.contains('/') {
                    continue;
                }

                let subresources = resource
                    .subresources
                    .iter()
                    .map(|sub| Subresource {
                        name: sub.subresource.clone(),
                        verbs: sub.verbs.clone(),
                    })
                    .collect();

                resources.push(ResourceMetadata {
                    group: group.metadata.name.clone(),
                    version: version.version.clone(),
                    kind: resource.response_kind.kind.clone(),
                    plural: resource.resource.clone(),
                    singular: resource
                        .singular_resource
                        .clone()
                        .unwrap_or_else(|| resource.resource.trim_end_matches('s').to_string()),
                    namespaced: resource.scope == "Namespaced",
                    verbs: resource.verbs.clone(),
                    subresources,
                    short_names: resource.short_names.clone(),
                });
            }
        }
    }

    Ok(resources)
}

/// Parse the discovery files of one release and return combined resource metadata
fn parse_discovery_files(dir: &Path) -> Result<Vec<ResourceMetadata>, Box<dyn std::error::Error>> {
    let mut resources = Vec::new();

    // Parse core API (v1)
    let core_resources = parse_core_api(dir)?;
    println!("Parsed {} core API resources", core_resources.len());
    resources.extend(core_resources);

    // Parse aggregated discovery (all other API groups)
    let aggregated_resources = parse_aggregated_discovery(dir)?;
    println!(
        "Parsed {} aggregated API resources",
        aggregated_resources.len()
    );
    resources.extend(aggregated_resources);

    Ok(resources)
}

/// Merge the resources of every release in `RELEASES`.
///
/// The newest release that serves a resource sets its position and metadata.
/// Older releases add the verbs, subresources, and short names that it lacks.
fn merge_releases() -> Result<Vec<ResourceMetadata>, Box<dyn std::error::Error>> {
    let mut merged: Vec<ResourceMetadata> = Vec::new();
    let mut index: HashMap<(String, String, String), usize> = HashMap::new();

    for tag in RELEASES {
        println!("Release {}:", tag);
        for resource in parse_discovery_files(&release_dir(DISCOVERY_DIR, tag))? {
            let key = (
                resource.group.clone(),
                resource.version.clone(),
                resource.kind.clone(),
            );
            let Some(&position) = index.get(&key) else {
                index.insert(key, merged.len());
                merged.push(resource);
                continue;
            };

            let existing = &mut merged[position];
            if existing.plural != resource.plural || existing.namespaced != resource.namespaced {
                eprintln!(
                    "Warning: {}/{}/{} has a different plural or scope in {}; keeping the newer one",
                    resource.group, resource.version, resource.kind, tag
                );
            }
            union(&mut existing.verbs, resource.verbs);
            union(&mut existing.short_names, resource.short_names);
            for sub in resource.subresources {
                match existing
                    .subresources
                    .iter_mut()
                    .find(|s| s.name == sub.name)
                {
                    Some(existing_sub) => union(&mut existing_sub.verbs, sub.verbs),
                    None => existing.subresources.push(sub),
                }
            }
        }
    }

    Ok(merged)
}

/// Append each item of `from` that `into` does not already contain.
fn union(into: &mut Vec<String>, from: Vec<String>) {
    for item in from {
        if !into.contains(&item) {
            into.push(item);
        }
    }
}

/// Template for generating discovery.rs
const TEMPLATE: &str = r#"// Auto-generated Kubernetes resource discovery metadata
//
// This file is generated by the discovery-gen binary and should not be edited manually.
// To regenerate: cargo run -p xtask --bin discovery-gen

/// Returns whether a resource is namespaced, or `None` if the resource is unknown.
pub fn is_namespaced(group: &str, version: &str, kind: &str) -> Option<bool> {
    match (group, version, kind) {
        {%- for resource in resources %}
        ("{{ resource.group }}", "{{ resource.version }}", "{{ resource.kind }}") => Some({{ resource.namespaced }}),
        {%- endfor %}
        _ => None,
    }
}

/// Returns the plural name for a resource, or `None` if the resource is unknown.
pub fn get_plural(group: &str, version: &str, kind: &str) -> Option<&'static str> {
    match (group, version, kind) {
        {%- for resource in resources %}
        ("{{ resource.group }}", "{{ resource.version }}", "{{ resource.kind }}") => Some("{{ resource.plural }}"),
        {%- endfor %}
        _ => None,
    }
}

/// Returns the singular name for a resource, or `None` if the resource is unknown.
pub fn get_singular(group: &str, version: &str, kind: &str) -> Option<&'static str> {
    match (group, version, kind) {
        {%- for resource in resources %}
        ("{{ resource.group }}", "{{ resource.version }}", "{{ resource.kind }}") => Some("{{ resource.singular }}"),
        {%- endfor %}
        _ => None,
    }
}

/// Returns the Kind for a resource given its plural name, or `None` if unknown.
pub fn plural_to_kind(group: &str, version: &str, plural: &str) -> Option<&'static str> {
    match (group, version, plural) {
        {%- for resource in resources %}
        ("{{ resource.group }}", "{{ resource.version }}", "{{ resource.plural }}") => Some("{{ resource.kind }}"),
        {%- endfor %}
        _ => None,
    }
}

/// Returns whether a resource has a given subresource.
#[allow(clippy::match_like_matches_macro)]
pub fn has_subresource(group: &str, version: &str, kind: &str, subresource: &str) -> bool {
    match (group, version, kind, subresource) {
        {%- for resource in resources %}
        {%- for sub in resource.subresources %}
        ("{{ resource.group }}", "{{ resource.version }}", "{{ resource.kind }}", "{{ sub.name }}") => true,
        {%- endfor %}
        {%- endfor %}
        _ => false,
    }
}

/// Returns the short names for a resource.
pub fn get_short_names(group: &str, version: &str, kind: &str) -> &'static [&'static str] {
    match (group, version, kind) {
        {%- for resource in resources %}
        {%- if resource.short_names | length > 0 %}
        ("{{ resource.group }}", "{{ resource.version }}", "{{ resource.kind }}") => &[{% for name in resource.short_names %}"{{ name }}"{% if not loop.last %}, {% endif %}{% endfor %}],
        {%- endif %}
        {%- endfor %}
        _ => &[],
    }
}

/// Returns whether a resource supports a given verb.
#[allow(clippy::match_like_matches_macro)]
pub fn supports_verb(group: &str, version: &str, kind: &str, verb: &str) -> bool {
    match (group, version, kind, verb) {
        {%- for resource in resources %}
        {%- for verb in resource.verbs %}
        ("{{ resource.group }}", "{{ resource.version }}", "{{ resource.kind }}", "{{ verb }}") => true,
        {%- endfor %}
        {%- endfor %}
        _ => false,
    }
}

/// Returns a static slice of all known resources as (group, version, kind, plural) tuples.
pub fn list_resources() -> &'static [(&'static str, &'static str, &'static str, &'static str)] {
    &[
        {%- for resource in resources %}
        ("{{ resource.group }}", "{{ resource.version }}", "{{ resource.kind }}", "{{ resource.plural }}"),
        {%- endfor %}
    ]
}
"#;

/// Generate discovery code from parsed resources
fn generate_discovery_code(
    resources: &[ResourceMetadata],
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut tera = Tera::default();
    tera.add_raw_template("discovery", TEMPLATE)?;

    let mut context = Context::new();
    context.insert("resources", resources);

    let rendered = tera.render("discovery", &context)?;
    fs::write(output_path, rendered)?;

    Ok(())
}
