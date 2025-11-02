# kube-fake-client

[![CI](https://github.com/ctxswitch/kube-fake-client-rs/workflows/CI/badge.svg)](https://github.com/ctxswitch/kube-fake-client-rs/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/kube-fake-client.svg)](https://crates.io/crates/kube-fake-client)
[![Documentation](https://docs.rs/kube-fake-client/badge.svg)](https://docs.rs/kube-fake-client)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

In-memory Kubernetes client for testing controllers and operators. Based on the [controller-runtime fake client](https://github.com/kubernetes-sigs/controller-runtime/tree/main/pkg/client/fake).

```toml
[dev-dependencies]
kube-fake-client = "0.1"
```

## Features

- **Full CRUD operations** - Create, read, update, patch, and delete resources
- **Status subresources** - Separate spec and status updates like real Kubernetes
- **Label and field selectors** - Filter resources with custom indexes
- **Namespace isolation** - Proper multi-namespace support
- **Resource version tracking** - Automatic versioning and conflict detection
- **YAML fixtures** - Load test data from files
- **Custom resources** - Works with CRDs and custom types
- **Interceptors** - Inject custom behavior for error simulation and validation

## Usage

```rust
use kube_fake_client::ClientBuilder;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, Patch, PatchParams};
use serde_json::json;

// Simple controller that adds a label to pods
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

    let client = ClientBuilder::new()
        .with_object(pod)
        .build()
        .await?;

    let pods: Api<Pod> = Api::namespaced(client, "default");
    let controller = PodController { api: pods.clone() };

    // Run the controller
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

## Examples

See [examples/](examples/) for more detailed examples including:
- Controller testing patterns
- Status subresource handling
- Custom resources (CRDs)
- YAML fixture loading
- Interceptors for error injection

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

**Stacked PRs**: This project supports stacked PRs with automatic stack visualization in PR descriptions. See the [Stacked PRs Workflow](.github/workflows/README.md#stacked-prs-workflow) documentation.

## License

Apache-2.0
