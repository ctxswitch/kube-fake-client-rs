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
