//! Resource registry for custom resource definitions (CRDs)
//!
//! This module provides a registry system for registering custom resources
//! with the fake client. Similar to how Kubernetes requires CRDs to be installed
//! before they can be used, the fake client requires custom resources to be
//! explicitly registered.

use kube::Resource;
use std::collections::HashMap;
use std::sync::RwLock;

/// Metadata for a registered resource type
#[derive(Debug, Clone)]
pub struct ResourceMetadata {
    /// The Kind name (e.g., "MyApp")
    pub kind: String,
    /// The API group (e.g., "example.com")
    pub group: String,
    /// The API version (e.g., "v1")
    pub version: String,
    /// The plural name (e.g., "myapps")
    pub plural: String,
    /// Whether the resource is namespaced
    pub namespaced: bool,
}

/// Registry for custom resource types
///
/// Stores metadata about registered CRDs to enable URL parsing and discovery.
/// This mimics real Kubernetes where CRDs must be installed before use.
#[derive(Debug, Default)]
pub struct ResourceRegistry {
    /// Lookup by (group, version, plural) -> ResourceMetadata
    /// Uses RwLock for interior mutability instead of Arc cloning
    resources: RwLock<HashMap<(String, String, String), ResourceMetadata>>,
}

impl ResourceRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            resources: RwLock::new(HashMap::new()),
        }
    }

    /// Register a resource type using its Resource trait implementation
    ///
    /// Extracts metadata from the type's Resource trait and stores it for lookup.
    pub fn register<K: Resource<DynamicType = ()>>(&self) {
        let kind = K::kind(&()).into_owned();
        let group = K::group(&()).into_owned();
        let version = K::version(&()).into_owned();
        let plural = K::plural(&()).into_owned();

        // Determine if namespaced by checking the Scope type
        // For now, we'll use a heuristic: if it has `fn namespaced()` we can call it
        // Otherwise default to true (most CRDs are namespaced)
        let namespaced = is_namespaced_resource();

        let metadata = ResourceMetadata {
            kind: kind.clone(),
            group: group.clone(),
            version: version.clone(),
            plural: plural.clone(),
            namespaced,
        };

        let key = (group, version, plural);
        self.resources
            .write()
            .expect("ResourceRegistry lock poisoned")
            .insert(key, metadata);
    }

    /// Look up a resource by (group, version, plural)
    pub fn lookup(&self, group: &str, version: &str, plural: &str) -> Option<ResourceMetadata> {
        self.resources
            .read()
            .expect("ResourceRegistry lock poisoned")
            .get(&(group.to_string(), version.to_string(), plural.to_string()))
            .cloned()
    }

    /// Get the Kind for a given plural name
    pub fn plural_to_kind(&self, group: &str, version: &str, plural: &str) -> Option<String> {
        self.lookup(group, version, plural).map(|m| m.kind)
    }

    /// Look up a resource by (group, version, kind)
    pub fn lookup_by_kind(
        &self,
        group: &str,
        version: &str,
        kind: &str,
    ) -> Option<ResourceMetadata> {
        self.resources
            .read()
            .expect("ResourceRegistry lock poisoned")
            .values()
            .find(|m| m.group == group && m.version == version && m.kind == kind)
            .cloned()
    }

    /// Get the plural for a given kind
    pub fn kind_to_plural(&self, group: &str, version: &str, kind: &str) -> Option<String> {
        self.lookup_by_kind(group, version, kind).map(|m| m.plural)
    }

    /// Check if a resource is namespaced
    pub fn is_namespaced(&self, group: &str, version: &str, kind: &str) -> Option<bool> {
        self.lookup_by_kind(group, version, kind)
            .map(|m| m.namespaced)
    }
}

/// Helper to determine if a Resource type is namespaced
fn is_namespaced_resource() -> bool {
    // Check if K::Scope implements the namespaced trait
    // For k8s-openapi types, K::Scope is either NamespaceResourceScope or ClusterResourceScope
    // For CustomResource, it's determined by the #[kube(namespaced)] attribute

    // Since we can't directly inspect the Scope type easily, we use a workaround:
    // Most CRDs are namespaced by default, cluster-scoped is the exception
    // The Resource trait doesn't directly expose this, so we default to true
    // Users can override this in the metadata if needed

    // TODO: Find a better way to extract this from the type system
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_empty() {
        let registry = ResourceRegistry::new();
        assert!(registry.lookup("example.com", "v1", "myapps").is_none());
    }
}
