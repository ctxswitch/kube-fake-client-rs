# kube-fake-client

[![CI](https://github.com/ctxswitch/kube-fake-client-rs/workflows/CI/badge.svg)](https://github.com/ctxswitch/kube-fake-client-rs/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/kube-fake-client.svg)](https://crates.io/crates/kube-fake-client)
[![Documentation](https://docs.rs/kube-fake-client/badge.svg)](https://docs.rs/kube-fake-client)
[![codecov](https://img.shields.io/codecov/c/github/ctxswitch/kube-fake-client-rs)](https://app.codecov.io/gh/ctxswitch/kube-fake-client-rs)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

`kube-fake-client` provides an in-memory backend for `kube::Api<K>`, so you can test Rust controllers and operators without a Kubernetes cluster. It is inspired by [controller-runtime's fake client](https://github.com/kubernetes-sigs/controller-runtime/tree/main/pkg/client/fake) for Go.

## Features

### Kubernetes behavior

- **`kube::Api<K>` operations:** Create, get, list, update, patch, and delete resources through the usual API.
- **Status subresources:** Configure separate spec and status updates for resource types that use them.
- **Resource versions:** Assign versions automatically and return conflicts for stale writes.
- **Namespace scope:** Keep namespaced resources isolated while also supporting cluster-scoped resources.
- **Selectors and indexes:** Filter with Kubernetes label and field selector syntax, including custom indexes.
- **Custom resources:** Register custom resource types and use them through `Api<K>`.

### Test setup and control

- **YAML fixtures:** Load single- or multi-document files into the client.
- **Interceptors:** Inject API errors, validate requests, or record operations.
- **OpenAPI validation:** Optionally validate resources against Kubernetes schemas with the `validation` feature.

## Installation

Add `kube-fake-client` as a development dependency in your `Cargo.toml`:

```toml
[dev-dependencies]
kube-fake-client = "0.3"
kube = { version = "4.2", features = ["client", "derive"] }
k8s-openapi = { version = "0.28", features = ["v1_36"] }
tokio = { version = "1.0", features = ["full"] }
```

### Kubernetes versions

Your `k8s-openapi` feature selects the Kubernetes version. `kube-fake-client` supports these versions:

- `v1_32` (or `earliest`): Kubernetes 1.32
- `v1_33`: Kubernetes 1.33
- `v1_34`: Kubernetes 1.34
- `v1_35`: Kubernetes 1.35
- `v1_36` (or `latest`): Kubernetes 1.36

### Optional OpenAPI validation

To check resources against their OpenAPI schemas at runtime, enable `validation`:

```toml
[dev-dependencies]
kube-fake-client = { version = "0.3", features = ["validation"] }
```

### Other dependencies

The test crate also needs:

- `kube` for `Api<K>` and other client types.
- `k8s-openapi` for Kubernetes resource types such as Pods and Deployments.
- `tokio` to run async tests.

## Usage

### Controller testing

Test a controller that adds a label to a Pod:

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

### Status subresources

Configure a `Deployment` status subresource:

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

### YAML fixtures

Load YAML files as test fixtures:

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

### Custom resources

Register and fetch a custom resource:

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

### Interceptors and error injection

Fail a Pod create request with an interceptor:

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

### Field selectors

Filter Pods by `metadata.name`:

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

For complete, runnable examples, see [`examples/`](examples/):

- [`basic_usage.rs`](examples/basic_usage.rs): CRUD, label and field selectors, and namespaced and cluster-scoped resources
- [`controller.rs`](examples/controller.rs): label management during reconciliation
- [`custom_resource.rs`](examples/custom_resource.rs): custom resource definitions
- [`status_controller.rs`](examples/status_controller.rs): separate spec and status updates
- [`fixture_loading.rs`](examples/fixture_loading.rs): YAML fixtures
- [`interceptors.rs`](examples/interceptors.rs): error injection and custom behavior
- [`schema_validations.rs`](examples/schema_validations.rs): OpenAPI schema validation (requires `validation`)

#### Run the examples

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

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, code style, tests, and the pull request process.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
