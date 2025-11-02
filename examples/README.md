# Examples

## Available Examples

- **basic_usage.rs** - Basic CRUD operations with namespaced and cluster-scoped resources
- **controller.rs** - Pod controller with testing
- **status_controller.rs** - Deployment status controller using status subresource
- **custom_resource.rs** - Using custom resources (CRDs)
- **fixture_loading.rs** - Loading fixtures from YAML files
- **interceptors.rs** - Interceptor usage patterns

## Running Examples

```bash
cargo run --example <example_name>
cargo test --example <example_name>
```
