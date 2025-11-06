//! Discovery metadata generator for kube-fake-client
//!
//! This binary generates Rust code from Kubernetes API discovery metadata.
//! Discovery data includes resource metadata, GVK mappings, verbs, and subresources.
//!
//! It can either use local JSON files or fetch them directly from the Kubernetes GitHub repo.
//!
//! # Usage
//!
//! Generate discovery code from local files:
//! ```bash
//! cargo run --bin discovery-gen
//! ```
//!
//! Update discovery data from Kubernetes GitHub repo:
//! ```bash
//! cargo run --bin discovery-gen -- --update
//! ```
//!
//! Target a specific Kubernetes version:
//! ```bash
//! cargo run --bin discovery-gen -- --update --tag v1.31.0
//! ```

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tera::{Context, Tera};

// Directory paths for Kubernetes API files
const DISCOVERY_DIR: &str = "kubernetes/api/discovery";

// GitHub repository configuration
const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com/kubernetes/kubernetes";
const USER_AGENT: &str = "kube-fake-client-discovery-gen";

#[derive(Parser, Debug)]
#[command(name = "discovery-gen")]
#[command(about = "Generate Kubernetes discovery code from JSON files", long_about = None)]
struct Args {
    /// Update discovery data from Kubernetes GitHub repository
    #[arg(short, long)]
    update: bool,

    /// Git tag or SHA to fetch from (default: master)
    #[arg(short, long, default_value = "master")]
    tag: String,

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

    // Ensure directories exist
    fs::create_dir_all(DISCOVERY_DIR)?;
    fs::create_dir_all(&args.output)?;

    // Check if discovery files exist
    let aggregated_path = Path::new(DISCOVERY_DIR).join("aggregated_v2.json");
    let core_path = Path::new(DISCOVERY_DIR).join("api__v1.json");
    let files_exist = aggregated_path.exists() && core_path.exists();

    // Fetch files if --update is specified or files don't exist
    if args.update || !files_exist {
        println!(
            "Fetching discovery data from Kubernetes GitHub repo (tag: {})...",
            args.tag
        );
        fetch_discovery_files(&args.tag)?;
        println!("Discovery data updated successfully");
    }

    // Parse and generate discovery code
    println!("Parsing discovery data...");
    let resources = parse_discovery_files()?;
    println!("Parsed {} resources", resources.len());

    println!("Generating discovery code...");
    let output_path = args.output.join("discovery.rs");
    generate_discovery_code(&resources, &output_path)?;
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

/// Fetch discovery files from Kubernetes GitHub repository
fn fetch_discovery_files(tag: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = create_http_client()?;

    // Fetch aggregated_v2.json
    let aggregated_url = format!(
        "{}/{}/api/discovery/aggregated_v2.json",
        GITHUB_RAW_BASE, tag
    );
    fetch_file(
        &client,
        &aggregated_url,
        &format!("{}/aggregated_v2.json", DISCOVERY_DIR),
    )?;

    // Fetch api__v1.json
    let core_url = format!("{}/{}/api/discovery/api__v1.json", GITHUB_RAW_BASE, tag);
    fetch_file(
        &client,
        &core_url,
        &format!("{}/api__v1.json", DISCOVERY_DIR),
    )?;

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
fn parse_core_api() -> Result<Vec<ResourceMetadata>, Box<dyn std::error::Error>> {
    let path = Path::new(DISCOVERY_DIR).join("api__v1.json");
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
fn parse_aggregated_discovery() -> Result<Vec<ResourceMetadata>, Box<dyn std::error::Error>> {
    let path = Path::new(DISCOVERY_DIR).join("aggregated_v2.json");
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

/// Parse all discovery files and return combined resource metadata
fn parse_discovery_files() -> Result<Vec<ResourceMetadata>, Box<dyn std::error::Error>> {
    let mut resources = Vec::new();

    // Parse core API (v1)
    let core_resources = parse_core_api()?;
    println!("Parsed {} core API resources", core_resources.len());
    resources.extend(core_resources);

    // Parse aggregated discovery (all other API groups)
    let aggregated_resources = parse_aggregated_discovery()?;
    println!(
        "Parsed {} aggregated API resources",
        aggregated_resources.len()
    );
    resources.extend(aggregated_resources);

    Ok(resources)
}

/// Template for generating discovery.rs
const TEMPLATE: &str = r#"//! Auto-generated Kubernetes resource discovery metadata
//!
//! This file is generated by the discovery-gen binary and should not be edited manually.
//! To regenerate: cargo run --bin discovery-gen

use std::collections::HashMap;
use once_cell::sync::Lazy;

/// Metadata about a Kubernetes resource type
#[derive(Debug, Clone)]
pub struct ResourceMetadata {
    pub group: &'static str,
    pub version: &'static str,
    pub kind: &'static str,
    pub plural: &'static str,
    pub singular: &'static str,
    pub namespaced: bool,
    pub verbs: &'static [&'static str],
    pub subresources: &'static [Subresource],
    pub short_names: &'static [&'static str],
}

/// Information about a subresource (status, scale, etc.)
#[derive(Debug, Clone)]
pub struct Subresource {
    pub name: &'static str,
    pub verbs: &'static [&'static str],
}

/// Global registry of all known Kubernetes resources
/// Keyed by (group, version, kind) tuple
pub static RESOURCE_REGISTRY: Lazy<HashMap<(&'static str, &'static str, &'static str), &'static ResourceMetadata>> = Lazy::new(|| {
    let mut m = HashMap::new();
    {% for resource in resources %}
    m.insert(("{{ resource.group }}", "{{ resource.version }}", "{{ resource.kind }}"), &RESOURCE_{{ loop.index0 }});
    {% endfor %}
    m
});

// Resource definitions
{% for resource in resources %}
static RESOURCE_{{ loop.index0 }}: ResourceMetadata = ResourceMetadata {
    group: "{{ resource.group }}",
    version: "{{ resource.version }}",
    kind: "{{ resource.kind }}",
    plural: "{{ resource.plural }}",
    singular: "{{ resource.singular }}",
    namespaced: {{ resource.namespaced }},
    verbs: &[{% for verb in resource.verbs %}"{{ verb }}"{% if not loop.last %}, {% endif %}{% endfor %}],
    subresources: &[
        {% for sub in resource.subresources %}
        Subresource {
            name: "{{ sub.name }}",
            verbs: &[{% for verb in sub.verbs %}"{{ verb }}"{% if not loop.last %}, {% endif %}{% endfor %}],
        },
        {% endfor %}
    ],
    short_names: &[{% for name in resource.short_names %}"{{ name }}"{% if not loop.last %}, {% endif %}{% endfor %}],
};
{% endfor %}

/// Look up resource metadata by GVK (Group, Version, Kind)
pub fn get_resource(group: &str, version: &str, kind: &str) -> Option<&'static ResourceMetadata> {
    RESOURCE_REGISTRY.get(&(group, version, kind)).copied()
}

/// Get all registered resources
pub fn all_resources() -> impl Iterator<Item = &'static ResourceMetadata> {
    RESOURCE_REGISTRY.values().copied()
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
