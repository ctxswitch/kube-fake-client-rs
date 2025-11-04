//! Builder for constructing fake clients with various options

use crate::client::{FakeClient, IndexerFunc};
use crate::client_utils::{extract_gvk, pluralize};
use crate::interceptor;
use crate::tracker::{GVK, GVR};
use crate::{Error, Result};
use kube::Resource;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Builder for creating fake clients
///
/// Provides a fluent API for constructing fake clients with various options:
/// - Initial objects
/// - Status subresources
/// - Indexes for field selectors
/// - Managed fields configuration
///
/// # Example
///
/// ```rust,no_run
/// use kube_fake_client::ClientBuilder;
/// use k8s_openapi::api::core::v1::Pod;
///
/// #[tokio::main]
/// async fn main() {
///     let client = ClientBuilder::new()
///         .with_return_managed_fields()
///         .build()
///         .await
///         .unwrap();
/// }
/// ```
pub struct ClientBuilder {
    initial_objects: Vec<Value>,
    with_status_subresource: Vec<GVK>,
    indexes: HashMap<GVK, HashMap<String, IndexerFunc>>,
    return_managed_fields: bool,
    fixture_dir: Option<PathBuf>,
    interceptors: Option<interceptor::Funcs>,
}

impl ClientBuilder {
    /// Create a new client builder
    pub fn new() -> Self {
        Self {
            initial_objects: Vec::new(),
            with_status_subresource: Vec::new(),
            indexes: HashMap::new(),
            return_managed_fields: false,
            fixture_dir: None,
            interceptors: None,
        }
    }

    /// Add initial objects to the fake client
    ///
    /// These objects will be created when the client is built.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use kube_fake_client::ClientBuilder;
    /// use k8s_openapi::api::core::v1::Pod;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut pod = Pod::default();
    /// pod.metadata.name = Some("test-pod".to_string());
    ///
    /// let client = ClientBuilder::new()
    ///     .with_object(pod)
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_object<K>(mut self, obj: K) -> Self
    where
        K: Resource + Serialize,
    {
        if let Ok(value) = serde_json::to_value(&obj) {
            self.initial_objects.push(value);
        }
        self
    }

    /// Add multiple initial objects
    pub fn with_objects<K>(mut self, objects: Vec<K>) -> Self
    where
        K: Resource + Serialize,
    {
        for obj in objects {
            if let Ok(value) = serde_json::to_value(&obj) {
                self.initial_objects.push(value);
            }
        }
        self
    }

    /// Add initial objects from JSON values
    pub fn with_runtime_objects(mut self, objects: Vec<Value>) -> Self {
        self.initial_objects.extend(objects);
        self
    }

    /// Enable status subresource for a specific resource type
    ///
    /// When a status subresource is enabled for a type:
    /// - Regular Update operations will not modify the status field
    /// - Status Update operations will not modify other fields
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use kube_fake_client::ClientBuilder;
    /// use k8s_openapi::api::core::v1::Pod;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = ClientBuilder::new()
    ///     .with_status_subresource::<Pod>()
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_status_subresource<K>(mut self) -> Self
    where
        K: Resource + Serialize + Default,
    {
        // Get GVK from a default instance
        let dummy = K::default();
        let dummy_value = serde_json::to_value(&dummy).expect("Failed to serialize default object");
        if let Ok(gvk) = extract_gvk(&dummy_value) {
            self.with_status_subresource.push(gvk);
        }
        self
    }

    /// Register an index for field selector support
    ///
    /// Indexes allow efficient filtering using field selectors in List operations.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use kube_fake_client::ClientBuilder;
    /// use k8s_openapi::api::core::v1::Pod;
    /// use std::sync::Arc;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = ClientBuilder::new()
    ///     .with_index::<Pod>(
    ///         "spec.nodeName",
    ///         Arc::new(|obj| {
    ///             obj.get("spec")
    ///                 .and_then(|s| s.get("nodeName"))
    ///                 .and_then(|n| n.as_str())
    ///                 .map(|s| vec![s.to_string()])
    ///                 .unwrap_or_default()
    ///         })
    ///     )
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_index<K>(mut self, field: impl Into<String>, indexer: IndexerFunc) -> Self
    where
        K: Resource + Serialize + Default,
    {
        // Get GVK from a default instance
        let dummy = K::default();
        let dummy_value = serde_json::to_value(&dummy).expect("Failed to serialize default object");
        if let Ok(gvk) = extract_gvk(&dummy_value) {
            let field = field.into();
            self.indexes.entry(gvk).or_default().insert(field, indexer);
        }

        self
    }

    /// Configure whether to return managed fields in responses
    ///
    /// By default, managed fields are stripped from responses to simplify testing.
    /// Enable this to test managed fields behavior.
    pub fn with_return_managed_fields(mut self) -> Self {
        self.return_managed_fields = true;
        self
    }

    /// Configure interceptor functions to customize client behavior
    ///
    /// Interceptors allow you to inject errors, implement custom logic, or track actions
    /// during tests. Each interceptor function can:
    /// - Return `Ok(Some(value))` to override the default behavior
    /// - Return `Ok(None)` to continue with the default behavior
    /// - Return `Err(e)` to inject an error
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use kube_fake_client::{ClientBuilder, interceptor};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = ClientBuilder::new()
    ///     .with_interceptor_funcs(
    ///         interceptor::Funcs::new().create(|ctx| {
    ///             if ctx.object.get("metadata")
    ///                 .and_then(|m| m.get("name"))
    ///                 .and_then(|n| n.as_str()) == Some("trigger-error") {
    ///                 return Err(kube_fake_client::Error::Internal("injected error".into()));
    ///             }
    ///             Ok(None)
    ///         })
    ///     )
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_interceptor_funcs(mut self, interceptors: interceptor::Funcs) -> Self {
        self.interceptors = Some(interceptors);
        self
    }

    /// Set the fixture directory for loading YAML fixtures
    ///
    /// This directory will be used as the base path for `load_fixture` calls.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use kube_fake_client::ClientBuilder;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = ClientBuilder::new()
    ///     .with_fixture_dir("fixtures")
    ///     .load_fixture("pods.yaml")?
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_fixture_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.fixture_dir = Some(dir.into());
        self
    }

    /// Load objects from a YAML fixture file
    ///
    /// Supports both single-document and multi-document YAML files (separated by `---`).
    /// Objects will be added to the initial objects list and created when the client is built.
    ///
    /// If a fixture directory was set with `with_fixture_dir`, the path is relative to that directory.
    /// Otherwise, the path is relative to the current working directory.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be read
    /// - The YAML cannot be parsed
    /// - The objects are invalid
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use kube_fake_client::ClientBuilder;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = ClientBuilder::new()
    ///     .with_fixture_dir("fixtures")
    ///     .load_fixture("pods.yaml")?
    ///     .load_fixture("deployments.yaml")?
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn load_fixture(mut self, path: impl AsRef<Path>) -> Result<Self> {
        let fixture_path = match &self.fixture_dir {
            Some(dir) => dir.join(path),
            None => path.as_ref().to_path_buf(),
        };

        let content = std::fs::read_to_string(&fixture_path).map_err(|e| {
            Error::Internal(format!(
                "Failed to read fixture file {:?}: {}",
                fixture_path, e
            ))
        })?;

        use serde::Deserialize;
        for document in serde_yaml::Deserializer::from_str(&content) {
            let mut value = Value::deserialize(document).map_err(|e| {
                Error::Internal(format!("Failed to parse YAML in {:?}: {}", fixture_path, e))
            })?;

            // Set default metadata if not present
            if let Some(metadata) = value.get_mut("metadata").and_then(|m| m.as_object_mut()) {
                // Set creation timestamp if not already set
                if !metadata.contains_key("creationTimestamp") {
                    metadata.insert(
                        "creationTimestamp".to_string(),
                        serde_json::to_value(chrono::Utc::now().to_rfc3339()).unwrap(),
                    );
                }

                // Set namespace to default if not specified
                if !metadata.contains_key("namespace") {
                    metadata.insert(
                        "namespace".to_string(),
                        Value::String("default".to_string()),
                    );
                }
            }

            self.initial_objects.push(value);
        }

        Ok(self)
    }

    /// Load objects from multiple YAML fixture files
    ///
    /// Loads all specified fixture files in order. Each file can contain single or
    /// multi-document YAML.
    ///
    /// # Errors
    ///
    /// Returns an error if any file cannot be read or parsed.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use kube_fake_client::ClientBuilder;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = ClientBuilder::new()
    ///     .with_fixture_dir("fixtures")
    ///     .load_fixtures(["pods.yaml", "deployments.yaml", "configmap.yaml"])?
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn load_fixtures<P>(mut self, paths: impl IntoIterator<Item = P>) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        for path in paths {
            self = self.load_fixture(path)?;
        }
        Ok(self)
    }

    /// Load objects from a YAML fixture file, panicking on error
    ///
    /// This is a convenience method that calls `load_fixture` and panics if it fails.
    /// Useful in test code where you want to fail fast if fixtures can't be loaded.
    ///
    /// # Panics
    ///
    /// Panics if the fixture file cannot be loaded or parsed.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use kube_fake_client::ClientBuilder;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let client = ClientBuilder::new()
    ///     .with_fixture_dir("fixtures")
    ///     .load_fixture_or_panic("pods.yaml")
    ///     .build()
    ///     .await
    ///     .unwrap();
    /// # }
    /// ```
    pub fn load_fixture_or_panic(self, path: impl AsRef<Path>) -> Self {
        self.load_fixture(path).expect("Failed to load fixture")
    }

    /// Load objects from multiple YAML fixture files, panicking on error
    ///
    /// This is a convenience method that calls `load_fixtures` and panics if it fails.
    ///
    /// # Panics
    ///
    /// Panics if any fixture file cannot be loaded or parsed.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use kube_fake_client::ClientBuilder;
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let client = ClientBuilder::new()
    ///     .with_fixture_dir("fixtures")
    ///     .load_fixtures_or_panic(["pods.yaml", "deployments.yaml"])
    ///     .build()
    ///     .await
    ///     .unwrap();
    /// # }
    /// ```
    pub fn load_fixtures_or_panic<P>(self, paths: impl IntoIterator<Item = P>) -> Self
    where
        P: AsRef<Path>,
    {
        self.load_fixtures(paths).expect("Failed to load fixtures")
    }

    /// Build a standard kube::Client with fake backend
    ///
    /// Returns a real `kube::Client` that works with standard `kube::Api<K>`,
    /// but routes all requests to an in-memory fake backend.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use kube_fake_client::ClientBuilder;
    /// use k8s_openapi::api::core::v1::Pod;
    /// use kube::Api;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// // Create a standard kube::Client with fake backend
    /// let client = ClientBuilder::new()
    ///     .build()
    ///     .await?;
    ///
    /// // Use with standard kube::Api
    /// let pods: Api<Pod> = Api::namespaced(client, "default");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if any initial objects fail to be created.
    pub async fn build(self) -> Result<kube::Client> {
        let fake_client = FakeClient {
            tracker: Arc::new(crate::tracker::ObjectTracker::new()),
            indexes: Arc::new(std::sync::RwLock::new(self.indexes)),
            return_managed_fields: self.return_managed_fields,
            interceptors: self.interceptors.map(Arc::new),
        };

        // Enable status subresources
        for gvk in self.with_status_subresource {
            fake_client.tracker.add_status_subresource(gvk);
        }

        // Add initial objects (using add() not create() to match Go's behavior)
        // This sets ResourceVersion to "999" instead of "1"
        for obj in self.initial_objects {
            let gvk = extract_gvk(&obj)?;
            let gvr = gvk_to_gvr(&gvk)?;
            let namespace = extract_namespace(&obj);

            fake_client
                .tracker
                .add(&gvr, &gvk, obj, &namespace)
                .map_err(|e| Error::Internal(format!("Failed to add initial object: {}", e)))?;
        }

        // Create the mock service
        let service = crate::mock_service::MockService::new(fake_client);

        // Create a kube::Client using the mock service
        let kube_client = kube::Client::new(service, "default");

        // Return the kube::Client
        Ok(kube_client)
    }
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert GVK to GVR (simplified - pluralizes kind)
fn gvk_to_gvr(gvk: &GVK) -> Result<GVR> {
    // Simple pluralization - in a real implementation, this would use
    // a REST mapper or API discovery
    let resource = pluralize(&gvk.kind);
    Ok(GVR::new(gvk.group.clone(), gvk.version.clone(), resource))
}

/// Extract namespace from object metadata
fn extract_namespace(obj: &Value) -> String {
    obj.get("metadata")
        .and_then(|m| m.get("namespace"))
        .and_then(|n| n.as_str())
        .unwrap_or("default")
        .to_string()
}
