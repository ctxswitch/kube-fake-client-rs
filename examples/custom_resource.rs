//! Using custom resources (CRDs)

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app1 = MyApp::new(
        "app1",
        MyAppSpec {
            replicas: 3,
            image: "nginx:latest".to_string(),
        },
    );
    app1.metadata.namespace = Some("default".to_string());

    let mut app2 = MyApp::new(
        "app2",
        MyAppSpec {
            replicas: 5,
            image: "redis:latest".to_string(),
        },
    );
    app2.metadata.namespace = Some("default".to_string());

    let client = ClientBuilder::new()
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

    let mut app3 = MyApp::new(
        "app3",
        MyAppSpec {
            replicas: 2,
            image: "postgres:14".to_string(),
        },
    );
    app3.metadata.namespace = Some("default".to_string());

    let created = api.create(&PostParams::default(), &app3).await?;
    println!("\nCreated new MyApp: {}", created.name_any());

    let retrieved = api.get("app3").await?;
    println!(
        "Retrieved app3 - replicas: {}, image: {}",
        retrieved.spec.replicas, retrieved.spec.image
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_custom_resource_crud() {
        let mut app = MyApp::new(
            "test-app",
            MyAppSpec {
                replicas: 1,
                image: "test:latest".to_string(),
            },
        );
        app.metadata.namespace = Some("default".to_string());

        let client = ClientBuilder::new().with_object(app).build().await.unwrap();

        let api: Api<MyApp> = Api::namespaced(client, "default");

        let retrieved = api.get("test-app").await.unwrap();
        assert_eq!(retrieved.spec.replicas, 1);
        assert_eq!(retrieved.spec.image, "test:latest");

        let apps = api.list(&ListParams::default()).await.unwrap();
        assert_eq!(apps.items.len(), 1);
    }
}
