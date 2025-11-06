//! Using custom resources (CRDs)
//!
//! Example demonstrating how to register and use custom resource definitions (CRDs)
//! with the fake client. This is useful for testing operators and controllers that
//! work with custom resources.

use kube::api::{Api, ListParams, PostParams};
use kube::{CustomResource, ResourceExt};
use kube_fake_client::ClientBuilder;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// CustomResource derive generates the MyApp type with metadata and spec fields
#[derive(CustomResource, Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[kube(group = "example.com", version = "v1", kind = "MyApp", namespaced)]
pub struct MyAppSpec {
    pub replicas: i32,
    pub image: String,
}

/// Create a MyApp resource with the given name, replicas, and image
fn create_app(name: &str, namespace: &str, replicas: i32, image: &str) -> MyApp {
    let mut app = MyApp::new(
        name,
        MyAppSpec {
            replicas,
            image: image.to_string(),
        },
    );
    app.metadata.namespace = Some(namespace.to_string());
    app
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app1 = create_app("app1", "default", 3, "nginx:latest");
    let app2 = create_app("app2", "default", 5, "redis:latest");

    // Register the custom resource type (like installing a CRD in a real cluster)
    let client = ClientBuilder::new()
        .with_resource::<MyApp>()
        .with_object(app1)
        .with_object(app2)
        .build()
        .await?;

    let api: Api<MyApp> = Api::namespaced(client, "default");

    let apps = api.list(&ListParams::default()).await?;
    println!("Found {} MyApp resources:", apps.items.len());
    for app in &apps.items {
        println!(
            "  {} - replicas: {}, image: {}",
            app.name_any(),
            app.spec.replicas,
            app.spec.image
        );
    }

    let app3 = create_app("app3", "default", 2, "postgres:14");

    let created = api.create(&PostParams::default(), &app3).await?;
    println!("\nCreated new MyApp: {}", created.name_any());

    let retrieved = api.get("app3").await?;
    println!(
        "Retrieved app3 - replicas: {}, image: {}",
        retrieved.spec.replicas, retrieved.spec.image
    );

    Ok(())
}
