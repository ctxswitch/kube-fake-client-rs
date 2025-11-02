//! Interceptor usage patterns

use k8s_openapi::api::core::v1::Pod;
use kube::api::PostParams;
use kube::Api;
use kube_fake_client::{interceptor, ClientBuilder, Error};
use serde_json::json;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Example 1: Error Injection ===");
    let client = create_client_with_error_injection().await?;
    let pods: Api<Pod> = Api::namespaced(client.clone(), "default");

    let mut normal_pod = Pod::default();
    normal_pod.metadata.name = Some("normal-pod".to_string());
    match pods.create(&PostParams::default(), &normal_pod).await {
        Ok(_) => println!("✓ Created normal-pod successfully"),
        Err(e) => println!("✗ Failed to create normal-pod: {}", e),
    }

    let mut error_pod = Pod::default();
    error_pod.metadata.name = Some("trigger-error".to_string());
    match pods.create(&PostParams::default(), &error_pod).await {
        Ok(_) => println!("✗ Should have failed!"),
        Err(e) => println!("✓ Expected error: {}", e),
    }

    println!("\n=== Example 2: Upsert Behavior ===");
    let client = create_client_with_custom_responses().await?;
    let pods: Api<Pod> = Api::namespaced(client.clone(), "default");

    let patch = json!({
        "metadata": {
            "labels": {
                "created-by": "upsert-interceptor"
            }
        }
    });

    match pods
        .patch(
            "auto-created-pod",
            &kube::api::PatchParams::default(),
            &kube::api::Patch::Merge(&patch),
        )
        .await
    {
        Ok(pod) => println!(
            "✓ Patch created nonexistent pod: {}",
            pod.metadata.name.unwrap_or_default()
        ),
        Err(e) => println!("✗ Unexpected error: {}", e),
    }

    match pods.get("auto-created-pod").await {
        Ok(_) => println!("✓ Verified pod exists after upsert"),
        Err(e) => println!("✗ Pod not found after upsert: {}", e),
    }

    println!("\n=== Example 3: Validation Interceptor ===");
    let client = create_client_with_validation().await?;
    let pods: Api<Pod> = Api::namespaced(client.clone(), "default");

    let mut protected_pod = Pod::default();
    protected_pod.metadata.name = Some("protected-pod".to_string());
    protected_pod.metadata.labels = Some(
        [("protected".to_string(), "true".to_string())]
            .iter()
            .cloned()
            .collect(),
    );
    pods.create(&PostParams::default(), &protected_pod).await?;

    match pods
        .delete("protected-pod", &kube::api::DeleteParams::default())
        .await
    {
        Ok(_) => println!("✗ Should not be able to delete protected pod!"),
        Err(e) => println!("✓ Protected pod deletion blocked: {}", e),
    }

    println!("\n=== Example 4: Call Tracking ===");
    let (client, create_count) = create_client_with_tracking().await?;
    let pods: Api<Pod> = Api::namespaced(client.clone(), "default");

    for i in 1..=3 {
        let mut pod = Pod::default();
        pod.metadata.name = Some(format!("tracked-pod-{}", i));
        pods.create(&PostParams::default(), &pod).await?;
    }

    println!(
        "✓ Created {} pods (tracked by interceptor)",
        create_count.lock().unwrap()
    );

    Ok(())
}

async fn create_client_with_error_injection() -> Result<kube::Client, Box<dyn std::error::Error>> {
    let client = ClientBuilder::new()
        .with_interceptor_funcs(interceptor::Funcs {
            // Return Err to simulate failures, Ok(None) to proceed with default behavior
            create: Some(Arc::new(|ctx| {
                if ctx
                    .object
                    .get("metadata")
                    .and_then(|m| m.get("name"))
                    .and_then(|n| n.as_str())
                    == Some("trigger-error")
                {
                    return Err(Error::Internal(
                        "injected creation error for testing".into(),
                    ));
                }
                Ok(None)
            })),
            ..Default::default()
        })
        .build()
        .await?;

    Ok(client)
}

async fn create_client_with_custom_responses() -> Result<kube::Client, Box<dyn std::error::Error>> {
    let client = ClientBuilder::new()
        .with_interceptor_funcs(interceptor::Funcs {
            // Return Ok(Some(value)) to override default behavior with a custom response
            patch: Some(Arc::new(|ctx| {
                use k8s_openapi::api::core::v1::Pod;

                match ctx.client.get::<Pod>(ctx.namespace, ctx.name) {
                    Ok(_) => Ok(None),
                    Err(Error::NotFound { .. }) => {
                        // Implement upsert: create the resource if it doesn't exist
                        let mut pod_value = json!({
                            "apiVersion": "v1",
                            "kind": "Pod",
                            "metadata": {
                                "name": ctx.name,
                                "namespace": ctx.namespace,
                            }
                        });
                        json_patch::merge(&mut pod_value, ctx.patch);

                        let pod: Pod = serde_json::from_value(pod_value)?;
                        let created = ctx.client.create(
                            ctx.namespace,
                            &pod,
                            &kube::api::PostParams::default(),
                        )?;
                        Ok(Some(serde_json::to_value(created)?))
                    }
                    Err(e) => Err(e),
                }
            })),
            ..Default::default()
        })
        .build()
        .await?;

    Ok(client)
}

async fn create_client_with_validation() -> Result<kube::Client, Box<dyn std::error::Error>> {
    let client = ClientBuilder::new()
        .with_interceptor_funcs(interceptor::Funcs {
            // Interceptors can validate operations before they execute
            delete: Some(Arc::new(|ctx| {
                use k8s_openapi::api::core::v1::Pod;

                match ctx.client.get::<Pod>(ctx.namespace, ctx.name) {
                    Ok(pod) => {
                        if let Some(labels) = &pod.metadata.labels {
                            if labels.get("protected") == Some(&"true".to_string()) {
                                return Err(Error::Internal(
                                    "Cannot delete protected resources".into(),
                                ));
                            }
                        }
                        Ok(None)
                    }
                    Err(e) => Err(e),
                }
            })),
            ..Default::default()
        })
        .build()
        .await?;

    Ok(client)
}

async fn create_client_with_tracking(
) -> Result<(kube::Client, Arc<std::sync::Mutex<u32>>), Box<dyn std::error::Error>> {
    use std::sync::Mutex;

    let create_count = Arc::new(Mutex::new(0));
    let create_count_clone = Arc::clone(&create_count);

    let client = ClientBuilder::new()
        .with_interceptor_funcs(interceptor::Funcs {
            // Interceptors can track API calls without modifying behavior
            create: Some(Arc::new(move |_ctx| {
                *create_count_clone.lock().unwrap() += 1;
                Ok(None)
            })),
            ..Default::default()
        })
        .build()
        .await?;

    Ok((client, create_count))
}
