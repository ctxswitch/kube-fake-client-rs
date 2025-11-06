//! Resource discovery information from Kubernetes API
//!
//! This module provides metadata about Kubernetes resources that is generated
//! from the official Kubernetes discovery API.
//!
//! The data is sourced from `kubernetes/api/discovery/` JSON files.
//! To update the generated lookup functions, run:
//! `cargo run --bin discovery-gen`
//!
//! # Architecture
//!
//! - This file (`src/discovery.rs`) - Stable API wrapper, add custom logic here
//! - `src/gen/discovery.rs` - Generated lookup functions, DO NOT EDIT

// Include the generated lookup functions
include!("gen/discovery.rs");

use crate::registry::ResourceRegistry;
use crate::tracker::{GVK, GVR};
use std::borrow::Cow;

/// Resource discovery information
///
/// This struct provides a stable API for querying Kubernetes resource metadata.
/// The underlying data comes from generated lookup functions in `src/gen/discovery.rs`.
///
/// # Adding Custom Logic
///
/// You can add additional methods here for CRD handling or custom discovery logic.
/// For example, you might add a fallback chain that tries static discovery first,
/// then falls back to heuristic pluralization for unknown CRDs.
pub struct Discovery;

impl Discovery {
    /// Check if a resource is namespaced (true) or cluster-scoped (false)
    ///
    /// Returns the exact value from discovery data for built-in resources.
    /// Returns `None` if the resource is not found (e.g., unregistered CRDs).
    pub fn is_namespaced(gvk: &GVK) -> Option<bool> {
        is_namespaced(&gvk.group, &gvk.version, &gvk.kind)
    }

    /// Get the plural name for a resource Kind
    ///
    /// Returns the exact plural from discovery data for built-in resources.
    /// Returns `None` if the resource is not found (e.g., unregistered CRDs).
    pub fn get_plural(gvk: &GVK) -> Option<&'static str> {
        get_plural(&gvk.group, &gvk.version, &gvk.kind)
    }

    /// Get the singular name for a resource Kind
    ///
    /// Returns the exact singular from discovery data for built-in resources.
    /// Returns `None` if the resource is not found (e.g., unregistered CRDs).
    pub fn get_singular(gvk: &GVK) -> Option<&'static str> {
        get_singular(&gvk.group, &gvk.version, &gvk.kind)
    }

    /// Get the Kind for a given plural resource name (reverse lookup from GVR)
    ///
    /// This is useful for converting URL paths (which use plural names) to Kinds.
    ///
    /// Returns None if the resource is not found in the discovery data.
    ///
    /// # Example
    /// ```
    /// use kube_fake_client::discovery::Discovery;
    ///
    /// // Convert "pods" -> "Pod"
    /// assert_eq!(Discovery::plural_to_kind("", "v1", "pods"), Some("Pod"));
    /// assert_eq!(Discovery::plural_to_kind("apps", "v1", "deployments"), Some("Deployment"));
    /// ```
    pub fn plural_to_kind(group: &str, version: &str, plural: &str) -> Option<&'static str> {
        plural_to_kind(group, version, plural)
    }

    /// Get the Kind for a given plural resource name, checking both built-in resources and registry
    ///
    /// This checks static discovery first, then checks the registry for registered CRDs.
    /// Returns None if the resource is not found in either location.
    ///
    /// This method should be used when parsing URLs to ensure CRDs are properly handled.
    ///
    /// # Example
    /// ```rust,no_run
    /// use kube_fake_client::discovery::Discovery;
    /// use kube_fake_client::registry::ResourceRegistry;
    ///
    /// let registry = ResourceRegistry::new();
    /// // For built-in resources, returns from discovery
    /// assert_eq!(Discovery::plural_to_kind_with_registry("", "v1", "pods", &registry), Some("Pod".into()));
    ///
    /// // For CRDs, must be registered first
    /// // Returns None if the CRD wasn't registered
    /// ```
    pub fn plural_to_kind_with_registry(
        group: &str,
        version: &str,
        plural: &str,
        registry: &ResourceRegistry,
    ) -> Option<Cow<'static, str>> {
        // First check static discovery (zero-cost for built-in resources)
        if let Some(kind) = plural_to_kind(group, version, plural) {
            return Some(Cow::Borrowed(kind));
        }

        // Check registry for registered CRDs
        registry
            .plural_to_kind(group, version, plural)
            .map(|s| Cow::Owned(s.to_string()))
    }

    /// Convert GVR to GVK using discovery data
    ///
    /// This is the reverse of `gvk_to_gvr` and is useful for converting URL paths to resource types.
    ///
    /// Returns None if the resource is not found in the discovery data.
    pub fn gvr_to_gvk(gvr: &GVR) -> Option<GVK> {
        plural_to_kind(&gvr.group, &gvr.version, &gvr.resource)
            .map(|kind| GVK::new(&gvr.group, &gvr.version, kind))
    }

    /// Convert GVR to GVK, checking both built-in resources and registry
    ///
    /// This checks static discovery first, then falls back to the registry for CRDs.
    /// Returns None if the resource is not found in either location.
    ///
    /// This method should be used when parsing URLs to ensure CRDs are properly handled.
    pub fn gvr_to_gvk_with_registry(gvr: &GVR, registry: &ResourceRegistry) -> Option<GVK> {
        Self::plural_to_kind_with_registry(&gvr.group, &gvr.version, &gvr.resource, registry)
            .map(|kind| GVK::new(&gvr.group, &gvr.version, kind.as_ref()))
    }

    /// Check if a resource has a specific subresource (e.g., "status", "scale")
    pub fn has_subresource(gvk: &GVK, subresource: &str) -> bool {
        has_subresource(&gvk.group, &gvk.version, &gvk.kind, subresource)
    }

    /// Get short names for a resource (e.g., "po" for Pod, "deploy" for Deployment)
    ///
    /// Returns an empty slice if the resource has no short names.
    pub fn get_short_names(gvk: &GVK) -> &'static [&'static str] {
        get_short_names(&gvk.group, &gvk.version, &gvk.kind)
    }

    /// Check if a resource supports a specific verb (e.g., "create", "delete", "watch")
    pub fn supports_verb(gvk: &GVK, verb: &str) -> bool {
        supports_verb(&gvk.group, &gvk.version, &gvk.kind, verb)
    }

    /// Convert GVK to GVR using discovery data
    ///
    /// For built-in resources, uses exact plural names from Kubernetes discovery.
    /// Returns None if the resource is not found (e.g., unregistered CRDs).
    pub fn gvk_to_gvr(gvk: &GVK) -> Option<GVR> {
        Self::get_plural(gvk).map(|plural| GVR::new(&gvk.group, &gvk.version, plural))
    }

    /// Convert GVK to GVR, checking both built-in resources and registry
    ///
    /// This checks static discovery first, then checks the registry for registered CRDs.
    /// Returns None if the resource is not found in either location.
    pub fn gvk_to_gvr_with_registry(gvk: &GVK, registry: &ResourceRegistry) -> Option<GVR> {
        // First check static discovery
        if let Some(plural) = Self::get_plural(gvk) {
            return Some(GVR::new(&gvk.group, &gvk.version, plural));
        }

        // Check registry for registered CRDs
        registry
            .kind_to_plural(&gvk.group, &gvk.version, &gvk.kind)
            .map(|plural| GVR::new(&gvk.group, &gvk.version, plural))
    }

    /// List all known built-in resources (for debugging/introspection)
    ///
    /// Returns a slice of (group, version, kind, plural) tuples for all resources
    /// in the discovery data. This does NOT include dynamically created CRDs.
    ///
    /// Useful for:
    /// - Debugging what resources are available
    /// - Building tools that need to discover available resources
    /// - Testing resource discovery logic
    ///
    /// # Example
    /// ```
    /// use kube_fake_client::discovery::Discovery;
    ///
    /// let resources = Discovery::list_all_resources();
    /// println!("Available resources: {}", resources.len());
    ///
    /// // Filter to just core v1 resources
    /// let core_v1: Vec<_> = resources.iter()
    ///     .filter(|(group, version, _, _)| group.is_empty() && *version == "v1")
    ///     .collect();
    /// ```
    pub fn list_all_resources(
    ) -> &'static [(&'static str, &'static str, &'static str, &'static str)] {
        list_resources()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pod_discovery() {
        let gvk = GVK::new("", "v1", "Pod");

        assert_eq!(Discovery::is_namespaced(&gvk), Some(true));
        assert_eq!(Discovery::get_plural(&gvk), Some("pods"));
        assert_eq!(Discovery::get_singular(&gvk), Some("pod"));
        assert!(Discovery::has_subresource(&gvk, "status"));
        assert!(Discovery::supports_verb(&gvk, "create"));
        assert!(Discovery::supports_verb(&gvk, "delete"));
        assert!(Discovery::supports_verb(&gvk, "watch"));

        let short_names = Discovery::get_short_names(&gvk);
        assert!(short_names.contains(&"po"));
    }

    #[test]
    fn test_deployment_discovery() {
        let gvk = GVK::new("apps", "v1", "Deployment");

        assert_eq!(Discovery::is_namespaced(&gvk), Some(true));
        assert_eq!(Discovery::get_plural(&gvk), Some("deployments"));
        assert!(Discovery::has_subresource(&gvk, "status"));
        assert!(Discovery::has_subresource(&gvk, "scale"));

        let short_names = Discovery::get_short_names(&gvk);
        assert!(short_names.contains(&"deploy"));
    }

    #[test]
    fn test_namespace_is_cluster_scoped() {
        let gvk = GVK::new("", "v1", "Namespace");

        assert_eq!(Discovery::is_namespaced(&gvk), Some(false));
        assert_eq!(Discovery::get_plural(&gvk), Some("namespaces"));
    }

    #[test]
    fn test_gvk_to_gvr() {
        let gvk = GVK::new("", "v1", "Service");
        let gvr = Discovery::gvk_to_gvr(&gvk).unwrap();

        assert_eq!(gvr.group, "");
        assert_eq!(gvr.version, "v1");
        assert_eq!(gvr.resource, "services");
    }

    #[test]
    fn test_unknown_resource_returns_none() {
        let gvk = GVK::new("example.com", "v1", "MyResource");

        // Should return None for unknown/unregistered resources
        assert_eq!(Discovery::is_namespaced(&gvk), None);
        assert_eq!(Discovery::get_plural(&gvk), None);
        assert_eq!(Discovery::get_singular(&gvk), None);
        assert_eq!(Discovery::gvk_to_gvr(&gvk), None);

        // Reverse lookup also returns None
        assert_eq!(
            Discovery::plural_to_kind("example.com", "v1", "myresources"),
            None
        );
    }

    #[test]
    fn test_plural_to_kind() {
        // Core v1 resources
        assert_eq!(Discovery::plural_to_kind("", "v1", "pods"), Some("Pod"));
        assert_eq!(
            Discovery::plural_to_kind("", "v1", "services"),
            Some("Service")
        );
        assert_eq!(
            Discovery::plural_to_kind("", "v1", "namespaces"),
            Some("Namespace")
        );
        assert_eq!(
            Discovery::plural_to_kind("", "v1", "configmaps"),
            Some("ConfigMap")
        );

        // Apps v1 resources
        assert_eq!(
            Discovery::plural_to_kind("apps", "v1", "deployments"),
            Some("Deployment")
        );
        assert_eq!(
            Discovery::plural_to_kind("apps", "v1", "statefulsets"),
            Some("StatefulSet")
        );
        assert_eq!(
            Discovery::plural_to_kind("apps", "v1", "daemonsets"),
            Some("DaemonSet")
        );

        // Unknown resource
        assert_eq!(Discovery::plural_to_kind("unknown", "v1", "foos"), None);
    }

    #[test]
    fn test_gvr_to_gvk() {
        // Core v1 Pod
        let gvr = GVR::new("", "v1", "pods");
        let gvk = Discovery::gvr_to_gvk(&gvr).unwrap();
        assert_eq!(gvk.group, "");
        assert_eq!(gvk.version, "v1");
        assert_eq!(gvk.kind, "Pod");

        // Apps v1 Deployment
        let gvr = GVR::new("apps", "v1", "deployments");
        let gvk = Discovery::gvr_to_gvk(&gvr).unwrap();
        assert_eq!(gvk.group, "apps");
        assert_eq!(gvk.version, "v1");
        assert_eq!(gvk.kind, "Deployment");

        // Unknown resource
        let gvr = GVR::new("unknown", "v1", "foos");
        assert_eq!(Discovery::gvr_to_gvk(&gvr), None);
    }

    #[test]
    fn test_gvk_gvr_round_trip() {
        // Test that we can convert GVK -> GVR -> GVK for known resources
        let original_gvk = GVK::new("", "v1", "Service");
        let gvr = Discovery::gvk_to_gvr(&original_gvk).unwrap();
        let recovered_gvk = Discovery::gvr_to_gvk(&gvr).unwrap();

        assert_eq!(original_gvk.group, recovered_gvk.group);
        assert_eq!(original_gvk.version, recovered_gvk.version);
        assert_eq!(original_gvk.kind, recovered_gvk.kind);
    }
}
