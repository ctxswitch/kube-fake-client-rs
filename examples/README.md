# Examples

## Basic Examples

- **basic_usage.rs** - Basic CRUD operations with namespaced and cluster-scoped resources
- **controller.rs** - Pod controller with testing
- **status_controller.rs** - Deployment status controller using status subresource
- **custom_resource.rs** - Using custom resources (CRDs)
- **fixture_loading.rs** - Loading fixtures from YAML files
- **interceptors.rs** - Interceptor usage patterns

## Validation Example

- **schema_validations.rs** - OpenAPI schema validation (requires `validation` feature)

## Running Examples

```bash
# Basic examples
cargo run --example <example_name>
cargo test --example <example_name>

# Validation example (requires feature flag)
cargo run --example schema_validations --features validation
cargo test --example schema_validations --features validation
```
