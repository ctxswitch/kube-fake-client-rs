//! Shared helpers for the code generators in `src/bin`.

use std::fs;
use std::path::{Path, PathBuf};

/// Kubernetes releases that the generated data covers, newest first.
///
/// Keep one release for each `v1_*` feature of the k8s-openapi version that the
/// root `Cargo.toml` uses. Data for each release lives in a directory named by its minor
/// version (for example `v1.36`), so a patch bump replaces the files in place.
pub const RELEASES: &[&str] = &["v1.36.5", "v1.35.9", "v1.34.12", "v1.33.13", "v1.32.13"];

const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com/kubernetes/kubernetes";

/// Returns the data directory for a release tag: `v1.36.5` in `base` is `base/v1.36`.
pub fn release_dir(base: &str, tag: &str) -> PathBuf {
    let minor = tag.rsplit_once('.').map_or(tag, |(minor, _)| minor);
    Path::new(base).join(minor)
}

/// Downloads `repo_path` at `tag` from the Kubernetes repository into `save_path`.
pub fn fetch_file(
    user_agent: &str,
    tag: &str,
    repo_path: &str,
    save_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(user_agent)
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let url = format!("{}/{}/{}", GITHUB_RAW_BASE, tag, repo_path);
    println!("Fetching {}...", url);
    let response = client.get(&url).send()?;

    if !response.status().is_success() {
        return Err(format!("Failed to fetch {}: HTTP {}", url, response.status()).into());
    }

    if let Some(parent) = save_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(save_path, response.text()?)?;
    Ok(())
}
