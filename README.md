# kube-fake-client

[![CI](https://github.com/ctxswitch/kube-fake-client-rs/workflows/CI/badge.svg)](https://github.com/ctxswitch/kube-fake-client-rs/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/kube-fake-client.svg)](https://crates.io/crates/kube-fake-client)
[![Documentation](https://docs.rs/kube-fake-client/badge.svg)](https://docs.rs/kube-fake-client)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

In-memory Kubernetes client for testing controllers and operators in Rust. Inspired by [controller-runtime's fake client](https://github.com/kubernetes-sigs/controller-runtime/tree/main/pkg/client/fake) from the Go ecosystem, this library provides a full-featured test client that mimics Kubernetes API behavior without requiring an actual cluster.

## Features

### Core Capabilities
- **Full CRUD Operations** - Create, read, update, patch, and delete resources with complete `kube::Api<K>` compatibility
- **Status Subresources** - Separate spec and status updates matching real Kubernetes behavior
- **Resource Version Tracking** - Automatic versioning with conflict detection for optimistic concurrency
- **Namespace Isolation** - Proper multi-namespace support with namespace-scoped and cluster-scoped resources

### Advanced Features
- **Label & Field Selectors** - Filter resources using standard Kubernetes selector syntax with custom indexing
- **YAML Fixtures** - Load test data from files (single or multi-document YAML)
- **Custom Resources (CRDs)** - First-class support for custom resource definitions
- **Interceptors** - Inject custom behavior for error simulation, validation, and action tracking
- **OpenAPI Schema Validation** - Optional runtime validation against Kubernetes OpenAPI specs (requires `validation` feature)

### Developer Experience
- **Drop-in Replacement** - Works seamlessly with existing `kube::Api<K>` code
- **Type-Safe** - Leverages Rust's type system for compile-time safety
- **Test-Friendly** - Designed specifically for unit and integration testing workflows

## Installation Instructions

### Basic Setup

Add `kube-fake-client` as a development dependency in your `Cargo.toml`:

```toml
[dev-dependencies]
kube-fake-client = "0.1"
kube = { version = "1.1", features = ["client", "derive"] }
k8s-openapi = { version = "0.25", features = ["v1_30"] }
tokio = { version = "1.0", features = ["full"] }
```

### With OpenAPI Validation (Optional)

To enable runtime schema validation:

```toml
[dev-dependencies]
kube-fake-client = { version = "0.1", features = ["validation"] }
```

### Dependencies Overview

The library requires:
- **kube** - Kubernetes client library for Rust (for `Api<K>` types and traits)
- **k8s-openapi** - Kubernetes API types (Pods, Deployments, etc.)
- **tokio** - Async runtime (required for async test functions)

All other dependencies are managed internally by the library.

## Usage

### Basic Controller Testing

Test a simple controller that adds labels to pods:

```rust
use kube_fake_client::ClientBuilder;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, Patch, PatchParams};
use serde_json::json;

// Controller that ensures pods have a "managed-by" label
struct PodController {
    api: Api<Pod>,
}

impl PodController {
    async fn reconcile(&self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let pod = self.api.get(name).await?;

        let needs_label = pod.metadata.labels.as_ref()
            .and_then(|labels| labels.get("managed-by"))
            .is_none();

        if needs_label {
            let patch = json!({
                "metadata": {
                    "labels": {
                        "managed-by": "pod-controller"
                    }
                }
            });
            self.api.patch(name, &PatchParams::default(), &Patch::Merge(&patch)).await?;
        }
        Ok(())
    }
}

#[tokio::test]
async fn test_controller_adds_label() -> Result<(), Box<dyn std::error::Error>> {
    // Create a pod without the managed-by label
    let mut pod = Pod::default();
    pod.metadata.name = Some("test-pod".to_string());
    pod.metadata.namespace = Some("default".to_string());

    // Build fake client with initial pod
    let client = ClientBuilder::new()
        .with_object(pod)
        .build()
        .await?;

    let pods: Api<Pod> = Api::namespaced(client, "default");
    let controller = PodController { api: pods.clone() };

    // Run controller reconciliation
    controller.reconcile("test-pod").await?;

    // Verify the label was added
    let updated = pods.get("test-pod").await?;
    assert_eq!(
        updated.metadata.labels.as_ref().unwrap().get("managed-by"),
        Some(&"pod-controller".to_string())
    );

    Ok(())
}
```

### Status Subresource Testing

Test controllers that update resource status separately from spec:

```rust
use k8s_openapi::api::apps::v1::Deployment;
use kube::api::Api;

#[tokio::test]
async fn test_status_update_isolation() -> Result<(), Box<dyn std::error::Error>> {
    let mut deployment = Deployment::default();
    deployment.metadata.name = Some("my-app".to_string());
    deployment.metadata.namespace = Some("default".to_string());

    // Enable status subresource for Deployment
    let client = ClientBuilder::new()
        .with_object(deployment)
        .with_status_subresource::<Deployment>()
        .build()
        .await?;

    let api: Api<Deployment> = Api::namespaced(client, "default");

    // Status updates don't affect spec, and vice versa
    // (implementation details omitted for brevity)

    Ok(())
}
```

### Loading YAML Fixtures

Load test data from YAML files:

```rust
#[tokio::test]
async fn test_with_fixtures() -> Result<(), Box<dyn std::error::Error>> {
    let client = ClientBuilder::new()
        .with_fixture_dir("tests/fixtures")
        .load_fixture("pods.yaml")?
        .load_fixture("deployments.yaml")?
        .build()
        .await?;

    let pods: Api<Pod> = Api::namespaced(client, "default");
    let pod_list = pods.list(&Default::default()).await?;

    assert!(!pod_list.items.is_empty());
    Ok(())
}
```

### Custom Resources (CRDs)

Test operators that work with custom resources:

```rust
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(group = "example.com", version = "v1", kind = "MyApp", namespaced)]
pub struct MyAppSpec {
    replicas: i32,
    image: String,
}

#[tokio::test]
async fn test_custom_resource() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = MyApp::new("my-app", MyAppSpec {
        replicas: 3,
        image: "nginx:latest".to_string(),
    });
    app.metadata.namespace = Some("default".to_string());

    // Register the CRD with the fake client
    let client = ClientBuilder::new()
        .with_resource::<MyApp>()
        .with_object(app)
        .build()
        .await?;

    let api: Api<MyApp> = Api::namespaced(client, "default");
    let retrieved = api.get("my-app").await?;

    assert_eq!(retrieved.spec.replicas, 3);
    Ok(())
}
```

### Error Injection with Interceptors

Simulate API errors for testing error handling:

```rust
use kube_fake_client::{ClientBuilder, interceptor, Error};

#[tokio::test]
async fn test_error_handling() -> Result<(), Box<dyn std::error::Error>> {
    let client = ClientBuilder::new()
        .with_interceptor_funcs(
            interceptor::Funcs::new().create(|ctx| {
                // Inject error for pods named "trigger-error"
                if ctx.object.get("metadata")
                    .and_then(|m| m.get("name"))
                    .and_then(|n| n.as_str()) == Some("trigger-error") {
                    return Err(Error::Internal("simulated error".into()));
                }
                Ok(None)
            })
        )
        .build()
        .await?;

    let pods: Api<Pod> = Api::namespaced(client, "default");

    let mut pod = Pod::default();
    pod.metadata.name = Some("trigger-error".to_string());

    // This create should fail due to interceptor
    let result = pods.create(&Default::default(), &pod).await;
    assert!(result.is_err());

    Ok(())
}
```

### Field Selectors

Filter resources using field selectors:

```rust
use kube::api::ListParams;

#[tokio::test]
async fn test_field_selectors() -> Result<(), Box<dyn std::error::Error>> {
    // Create pods and setup client (omitted for brevity)

    let pods: Api<Pod> = Api::namespaced(client, "default");

    // Filter by metadata.name (universally supported)
    let filtered = pods
        .list(&ListParams::default().fields("metadata.name=my-pod"))
        .await?;

    assert_eq!(filtered.items.len(), 1);
    Ok(())
}
```

### Examples

The [`examples/`](examples/) directory contains comprehensive examples demonstrating various patterns:

- **[basic_usage.rs](examples/basic_usage.rs)** - CRUD operations, label/field selectors, namespaced and cluster-scoped resources
- **[controller.rs](examples/controller.rs)** - Controller testing pattern with label management
- **[custom_resource.rs](examples/custom_resource.rs)** - Working with custom resource definitions (CRDs)
- **[status_controller.rs](examples/status_controller.rs)** - Status subresource handling and separation
- **[fixture_loading.rs](examples/fixture_loading.rs)** - Loading test data from YAML files
- **[interceptors.rs](examples/interceptors.rs)** - Error injection and custom behavior
- **[schema_validations.rs](examples/schema_validations.rs)** - Runtime OpenAPI schema validation (requires `validation` feature)

#### Running Examples

```bash
# Run a specific example
cargo run --example basic_usage
cargo run --example controller
cargo run --example custom_resource

# Run example with validation feature
cargo run --example schema_validations --features validation

# Run all examples
for example in basic_usage controller custom_resource fixture_loading \
               status_controller interceptors; do
    cargo run --example $example
done
```

## Contributing

Contributions are welcome! This project aims to closely follow the behavior of [controller-runtime's fake client](https://github.com/kubernetes-sigs/controller-runtime) while providing an idiomatic Rust experience.

Please see [CONTRIBUTING.md](CONTRIBUTING.md) for:
- Development setup
- Code style guidelines
- Testing requirements
- Pull request process

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
