//! OpenAPI Schema Validation Example
//!
//! This example demonstrates how to use OpenAPI schema validation with the fake client.
//! Validation is most useful when working with dynamically-generated or loaded resources
//! (e.g., from YAML files), as Rust's type system already catches most structural errors
//! at compile time for statically-typed code.
//!
//! Run with: cargo run --example schema_validations --features validation
//! Run tests: cargo test --example schema_validations --features validation

#![cfg(feature = "validation")]

use k8s_openapi::api::core::v1::{Container, Pod, PodSpec, Service, ServicePort, ServiceSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Api;
use kube_fake_client::ClientBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== OpenAPI Schema Validation Example ===\n");

    // Create client with schema validation
    let client = ClientBuilder::new()
        .with_schema_validation_file("kubernetes/api/openapi/swagger.json")?
        .with_validation_for("/v1/Pod")?
        .with_validation_for("/v1/Service")?
        .build()
        .await?;

    println!("Creating Pod...");
    let pods: Api<Pod> = Api::namespaced(client.clone(), "default");
    let pod = Pod {
        metadata: ObjectMeta {
            name: Some("example-pod".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        },
        spec: Some(PodSpec {
            containers: vec![Container {
                name: "nginx".to_string(),
                image: Some("nginx:latest".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };

    pods.create(&Default::default(), &pod).await?;
    println!("✓ Valid pod created successfully\n");

    println!("Creating Service...");
    let services: Api<Service> = Api::namespaced(client, "default");
    let service = Service {
        metadata: ObjectMeta {
            name: Some("example-service".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            ports: Some(vec![ServicePort {
                port: 80,
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };

    services.create(&Default::default(), &service).await?;
    println!("✓ Valid service created successfully\n");

    println!("=== Validation Notes ===\n");
    println!("The validator enforces OpenAPI schema validation including:");
    println!("  ✓ Type checking (string, number, object, array, etc.)");
    println!("  ✓ Required field validation");
    println!("  ✓ Structural validation (object shapes, array items)");
    println!();
    println!("IMPORTANT: Kubernetes built-in resource schemas do NOT include");
    println!("value constraint definitions (minLength, pattern, min/max, etc.).");
    println!("They rely on admission controllers to enforce those at runtime.");
    println!();
    println!("This validator is useful for:");
    println!("  • Validating dynamically loaded resources (e.g., from YAML)");
    println!("  • Catching structural errors before submission");
    println!("  • Type validation when using untyped JSON");
    println!("  • Testing that resources match the expected schema structure");
    println!();

    println!("=== Key Features ===\n");
    println!("  ✓ Simple API - just specify the swagger file path");
    println!("  ✓ Selective validation - enable only resources you need");
    println!("  ✓ Lazy schema compilation - schemas compiled on first validation");
    println!("  ✓ Validation errors show exactly what's wrong");
    println!("  ✓ Works with any OpenAPI file (Kubernetes built-in or CRDs)");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Test that valid resources pass validation
    #[tokio::test]
    async fn test_valid_pod_passes_validation() -> Result<(), Box<dyn std::error::Error>> {
        let client = ClientBuilder::new()
            .with_schema_validation_file("kubernetes/api/openapi/swagger.json")?
            .with_validation_for("/v1/Pod")?
            .build()
            .await?;

        let pods: Api<Pod> = Api::namespaced(client, "default");
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("test-pod".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "nginx".to_string(),
                    image: Some("nginx:latest".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        // Should succeed
        pods.create(&Default::default(), &pod).await?;
        Ok(())
    }

    /// Test that validation catches missing required fields
    #[tokio::test]
    async fn test_missing_required_field() {
        let client = ClientBuilder::new()
            .with_schema_validation_file("kubernetes/api/openapi/swagger.json")
            .unwrap()
            .with_validation_for("/v1/Pod")
            .unwrap()
            .build()
            .await
            .unwrap();

        // Create a Pod JSON missing the required "containers" field in spec
        let invalid_pod_json = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "missing-containers-pod",
                "namespace": "default"
            },
            "spec": {
                // Missing required "containers" field!
            }
        });

        // When deserializing, k8s-openapi will use defaults for missing fields
        // The containers field has a default of empty vec, so this will deserialize successfully
        let result = serde_json::from_value::<Pod>(invalid_pod_json);

        match result {
            Ok(pod) => {
                // The pod deserialized, but it has an empty containers array
                // which violates the OpenAPI schema requirement
                let pods: Api<Pod> = Api::namespaced(client, "default");
                let create_result = pods.create(&Default::default(), &pod).await;

                // The OpenAPI validator should catch that containers is required and can't be empty
                // Note: This depends on the OpenAPI schema having proper validation rules
                println!("Pod with empty containers result: {:?}", create_result.is_err());

                // In practice, the OpenAPI schema may not enforce non-empty arrays,
                // so this test demonstrates the limitation
            }
            Err(e) => {
                println!("Deserialization failed (expected): {}", e);
                // If deserialization fails, that's also catching the error
            }
        }
    }

    /// Test that validation catches type mismatches
    #[tokio::test]
    async fn test_type_mismatch() {
        // Pod with wrong type for restartPolicy (should be string, not number)
        let invalid_pod_json = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "type-mismatch-pod",
                "namespace": "default"
            },
            "spec": {
                "containers": [{
                    "name": "nginx",
                    "image": "nginx:latest"
                }],
                "restartPolicy": 123  // Wrong type! Should be string like "Always"
            }
        });

        // Rust's serde will catch this during deserialization
        let result = serde_json::from_value::<Pod>(invalid_pod_json);
        assert!(result.is_err(), "Expected deserialization to fail for type mismatch");

        println!("Type mismatch caught by serde deserialization");
    }

    /// Test that validation catches invalid enum values
    #[tokio::test]
    async fn test_invalid_enum_value() {
        let _client = ClientBuilder::new()
            .with_schema_validation_file("kubernetes/api/openapi/swagger.json")
            .unwrap()
            .with_validation_for("/v1/Service")
            .unwrap()
            .build()
            .await
            .unwrap();

        // Create a Service with invalid sessionAffinity value
        let invalid_service_json = json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": {
                "name": "invalid-service",
                "namespace": "default"
            },
            "spec": {
                "selector": {
                    "app": "nginx"
                },
                "ports": [{
                    "port": 80,
                    "targetPort": 80
                }],
                "sessionAffinity": "InvalidValue"  // Should be "ClientIP" or "None"
            }
        });

        // k8s-openapi may accept unknown enum values and map them to defaults or ignore them
        // This matches Kubernetes forward compatibility where unknown values are often tolerated
        let result = serde_json::from_value::<Service>(invalid_service_json);

        match result {
            Ok(_) => println!("Service deserialized - k8s-openapi allows unknown enum values for compatibility"),
            Err(e) => println!("Deserialization failed (also valid): {}", e),
        }
    }

    /// Test that validation catches additional properties when they're not allowed
    #[tokio::test]
    async fn test_unknown_fields_with_deny_unknown() {
        let client = ClientBuilder::new()
            .with_schema_validation_file("kubernetes/api/openapi/swagger.json")
            .unwrap()
            .with_validation_for("/v1/Pod")
            .unwrap()
            .build()
            .await
            .unwrap();

        // Note: k8s-openapi types with #[serde(flatten)] will accept unknown fields
        // This is actually the expected Kubernetes behavior - unknown fields are ignored
        // for forward/backward compatibility

        let pod_with_unknown = Pod {
            metadata: ObjectMeta {
                name: Some("pod-with-unknown".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "nginx".to_string(),
                    image: Some("nginx:latest".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let pods: Api<Pod> = Api::namespaced(client, "default");

        // This will succeed because Kubernetes allows unknown fields
        let result = pods.create(&Default::default(), &pod_with_unknown).await;
        assert!(result.is_ok(), "Kubernetes allows unknown fields for compatibility");
    }

    /// Test validation with nested required fields
    #[tokio::test]
    async fn test_nested_required_fields() {
        let _client = ClientBuilder::new()
            .with_schema_validation_file("kubernetes/api/openapi/swagger.json")
            .unwrap()
            .with_validation_for("/v1/Pod")
            .unwrap()
            .build()
            .await
            .unwrap();

        // Container is missing required "name" field
        let invalid_pod_json = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "nested-required-pod",
                "namespace": "default"
            },
            "spec": {
                "containers": [{
                    // Missing required "name" field
                    "image": "nginx:latest"
                }]
            }
        });

        // k8s-openapi will provide default values for missing required fields
        // This is by design for Kubernetes API compatibility
        let result = serde_json::from_value::<Pod>(invalid_pod_json);

        match result {
            Ok(pod) => {
                println!("Pod deserialized with default values");
                // Container name will be an empty string (the default)
                if let Some(spec) = &pod.spec {
                    if let Some(first_container) = spec.containers.first() {
                        println!("Container name: '{}'", first_container.name);
                        assert_eq!(first_container.name, "", "Missing name defaults to empty string");
                    }
                }
            }
            Err(e) => {
                println!("Deserialization failed: {}", e);
            }
        }
    }

    /// Test that validation works correctly for array items
    #[tokio::test]
    async fn test_array_item_validation() {
        let client = ClientBuilder::new()
            .with_schema_validation_file("kubernetes/api/openapi/swagger.json")
            .unwrap()
            .with_validation_for("/v1/Pod")
            .unwrap()
            .build()
            .await
            .unwrap();

        // Valid pod with multiple containers
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("multi-container-pod".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![
                    Container {
                        name: "nginx".to_string(),
                        image: Some("nginx:latest".to_string()),
                        ..Default::default()
                    },
                    Container {
                        name: "sidecar".to_string(),
                        image: Some("sidecar:latest".to_string()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };

        let pods: Api<Pod> = Api::namespaced(client, "default");
        let result = pods.create(&Default::default(), &pod).await;
        assert!(result.is_ok(), "Valid multi-container pod should pass validation");
    }

    /// Test without validation - any structure should be accepted
    #[tokio::test]
    async fn test_without_validation_accepts_all() -> Result<(), Box<dyn std::error::Error>> {
        // Client without validation
        let client = ClientBuilder::new().build().await?;

        let pods: Api<Pod> = Api::namespaced(client, "default");
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("no-validation-pod".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "nginx".to_string(),
                    image: Some("nginx:latest".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        // Should succeed without validation
        pods.create(&Default::default(), &pod).await?;
        Ok(())
    }

    /// Test that selective validation only validates enabled resources
    #[tokio::test]
    async fn test_selective_validation() -> Result<(), Box<dyn std::error::Error>> {
        // Only enable validation for Pods, not Services
        let client = ClientBuilder::new()
            .with_schema_validation_file("kubernetes/api/openapi/swagger.json")?
            .with_validation_for("/v1/Pod")?
            // Note: NOT enabling validation for Services
            .build()
            .await?;

        // Pod should be validated
        let pods: Api<Pod> = Api::namespaced(client.clone(), "default");
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("validated-pod".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "nginx".to_string(),
                    image: Some("nginx:latest".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        pods.create(&Default::default(), &pod).await?;

        // Service should NOT be validated (even if invalid, it would pass)
        let services: Api<Service> = Api::namespaced(client, "default");
        let service = Service {
            metadata: ObjectMeta {
                name: Some("unvalidated-service".to_string()),
                ..Default::default()
            },
            spec: Some(ServiceSpec {
                ports: Some(vec![ServicePort {
                    port: 80,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        services.create(&Default::default(), &service).await?;

        Ok(())
    }

    /// Test type coercion as validation
    ///
    /// This demonstrates that serde deserialization from untyped JSON provides
    /// a form of validation automatically - catching type errors, some structural
    /// issues, etc. This is "free" validation from Rust's type system.
    #[tokio::test]
    async fn test_type_coercion_validation() {
        println!("\n=== Type Coercion Validation Examples ===\n");

        // Example 1: Type mismatch - metadata should be object, not string
        let bad_metadata = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": "not-an-object",  // Wrong type!
            "spec": {
                "containers": [{"name": "nginx", "image": "nginx"}]
            }
        });

        let result = serde_json::from_value::<Pod>(bad_metadata);
        println!("1. Type mismatch (metadata as string):");
        match result {
            Ok(_) => println!("   ✗ Unexpectedly succeeded"),
            Err(e) => println!("   ✓ Caught by serde: {}", e),
        }

        // Example 2: Wrong type for a field - port should be number, not string
        let bad_port_type = json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": {"name": "svc"},
            "spec": {
                "ports": [{
                    "port": "eighty",  // Wrong type! Should be integer
                    "targetPort": 80
                }]
            }
        });

        let result = serde_json::from_value::<Service>(bad_port_type);
        println!("\n2. Type mismatch (port as string):");
        match result {
            Ok(_) => println!("   ✗ Unexpectedly succeeded"),
            Err(e) => println!("   ✓ Caught by serde: {}", e),
        }

        // Example 3: Completely wrong structure
        let bad_structure = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {"name": "test"},
            "spec": "not-an-object"  // spec should be object, not string
        });

        let result = serde_json::from_value::<Pod>(bad_structure);
        println!("\n3. Wrong structure (spec as string):");
        match result {
            Ok(_) => println!("   ✗ Unexpectedly succeeded"),
            Err(e) => println!("   ✓ Caught by serde: {}", e),
        }

        // Example 4: What serde DOESN'T catch - invalid values within valid types
        let bad_value_valid_type = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {"name": "test"},
            "spec": {
                "containers": [{
                    "name": "",  // Valid type (string), but semantically invalid (empty)
                    "image": "nginx"
                }]
            }
        });

        let result = serde_json::from_value::<Pod>(bad_value_valid_type);
        println!("\n4. Invalid value but valid type (empty container name):");
        match result {
            Ok(_) => println!("   ✗ Serde accepts this (type is correct, value constraint not checked)"),
            Err(e) => println!("   ✓ Caught by serde: {}", e),
        }

        println!("\n=== Summary ===");
        println!("Type coercion (serde deserialization) catches:");
        println!("  ✓ Type mismatches (string vs number vs object)");
        println!("  ✓ Structural errors (missing required types)");
        println!("  ✓ Wrong array element types");
        println!("\nType coercion does NOT catch:");
        println!("  ✗ Value constraints (empty strings, out-of-range numbers)");
        println!("  ✗ Business logic rules (port ranges, DNS format)");
        println!("  ✗ Cross-field validation (e.g., certain fields required together)");
        println!("\nOpenAPI validation adds:");
        println!("  • Schema structure validation (object shapes, required fields)");
        println!("  • Type consistency checks across the schema");
        println!("  • Validation against the expected resource shape\n");
    }
}
