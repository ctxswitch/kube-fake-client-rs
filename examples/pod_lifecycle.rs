//! Moving a Pod through its phases
//!
//! The fake client has no kubelet, scheduler or controllers, so nothing sets a Pod's phase
//! on its own. A created Pod has no phase until the test writes one, the way a kubelet
//! would on a real cluster.

use k8s_openapi::api::core::v1::{Pod, PodStatus};
use kube::api::{Api, ListParams, Patch, PatchParams, PostParams};
use kube_fake_client::{interceptor, ClientBuilder};
use serde_json::json;

fn pod_with_phase(name: &str, phase: &str) -> Pod {
    let mut pod = Pod::default();
    pod.metadata.name = Some(name.to_string());
    pod.metadata.namespace = Some("default".to_string());
    pod.status = Some(PodStatus {
        phase: Some(phase.to_string()),
        ..Default::default()
    });
    pod
}

fn phase_of(pod: &Pod) -> Option<&str> {
    pod.status.as_ref()?.phase.as_deref()
}

/// Stands in for the kubelet. Writes through the status subresource, the only way to
/// change a phase after a create.
async fn set_phase(pods: &Api<Pod>, name: &str, phase: &str) -> Result<Pod, kube::Error> {
    pods.patch_status(
        name,
        &PatchParams::default(),
        &Patch::Merge(json!({ "status": { "phase": phase } })),
    )
    .await
}

async fn running_pods(pods: &Api<Pod>) -> Result<Vec<String>, kube::Error> {
    let params = ListParams::default().fields("status.phase=Running");
    let mut names: Vec<String> = pods
        .list(&params)
        .await?
        .items
        .into_iter()
        .filter_map(|pod| pod.metadata.name)
        .collect();
    names.sort();
    Ok(names)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `with_object` and fixtures keep their status, so a test can start a Pod in any phase
    let client = ClientBuilder::new()
        .with_object(pod_with_phase("seeded", "Running"))
        .build()
        .await?;
    let pods: Api<Pod> = Api::namespaced(client, "default");

    // A create drops the status it submits, and a real cluster would report `Pending` here
    let created = pods
        .create(&PostParams::default(), &pod_with_phase("web", "Running"))
        .await?;
    assert_eq!(phase_of(&created), None);
    println!("created web: {:?}", phase_of(&created));

    // Nothing sets it, so a controller waiting on `Running` needs the test to act
    assert_eq!(phase_of(&pods.get("web").await?), None);
    assert_eq!(running_pods(&pods).await?, ["seeded"]);

    // A write to the main resource keeps the stored status
    let mut stale = pods.get("web").await?;
    stale.status = Some(PodStatus {
        phase: Some("Running".to_string()),
        ..Default::default()
    });
    let replaced = pods.replace("web", &PostParams::default(), &stale).await?;
    assert_eq!(phase_of(&replaced), None);
    println!("after replace: {:?}", phase_of(&replaced));
    let patched = pods
        .patch(
            "web",
            &PatchParams::default(),
            &Patch::Merge(json!({ "status": { "phase": "Running" } })),
        )
        .await?;
    assert_eq!(phase_of(&patched), None);
    println!("after patch: {:?}", phase_of(&patched));

    // A status write changes it
    let running = set_phase(&pods, "web", "Running").await?;
    assert_eq!(phase_of(&running), Some("Running"));
    assert_eq!(running_pods(&pods).await?, ["seeded", "web"]);
    println!("after status write: {:?}", phase_of(&running));

    // The fake does not check the order of phases
    let succeeded = set_phase(&pods, "web", "Succeeded").await?;
    assert_eq!(phase_of(&succeeded), Some("Succeeded"));
    println!("finished: {:?}", phase_of(&succeeded));

    // Code that reads the phase from the create response leaves no gap for a status write,
    // so a `create` interceptor stands in for the API server and supplies it
    let client = ClientBuilder::new()
        .with_interceptor_funcs(interceptor::Funcs::new().create(|ctx| {
            if ctx.object["kind"] != "Pod" {
                return Ok(None);
            }
            let pod: Pod = serde_json::from_value(ctx.object.clone())?;
            let mut created = ctx.client.create(ctx.namespace, &pod, ctx.params)?;
            created.status = Some(PodStatus {
                phase: Some("Pending".to_string()),
                ..Default::default()
            });
            let updated = ctx
                .client
                .update_status(ctx.namespace, &created, ctx.params)?;
            Ok(Some(serde_json::to_value(updated)?))
        }))
        .build()
        .await?;
    let pods: Api<Pod> = Api::namespaced(client, "default");
    let created = pods
        .create(&PostParams::default(), &pod_with_phase("gated", "Running"))
        .await?;
    assert_eq!(phase_of(&created), Some("Pending"));
    assert_eq!(phase_of(&pods.get("gated").await?), Some("Pending"));
    println!("created with an interceptor: {:?}", phase_of(&created));

    Ok(())
}
