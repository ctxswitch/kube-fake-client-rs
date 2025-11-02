//! Mock tower service that routes HTTP requests to the fake client

use crate::client::FakeClient;
use crate::client_utils::extract_gvk;
use crate::interceptor;
use crate::tracker::{GVK, GVR};
use bytes::Bytes;
use futures::future::{BoxFuture, FutureExt};
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use kube::api::{ListParams, PatchParams, PostParams};
use kube::client::Body as KubeBody;
use serde_json::Value;
use std::task::{Context, Poll};
use tower::Service;

/// Parsed Kubernetes API path information
struct ParsedPath {
    group: Option<String>,
    version: String,
    namespace: String,
    resource: String,
    name: Option<String>,
}

/// Mock HTTP service that routes requests to the fake client backend
#[derive(Clone)]
pub struct MockService {
    client: FakeClient,
}

impl MockService {
    pub fn new(client: FakeClient) -> Self {
        Self { client }
    }

    /// Parse URL path to extract API info
    /// Examples:
    /// - /api/v1/namespaces/default/pods
    /// - /api/v1/namespaces/default/pods/my-pod
    /// - /apis/apps/v1/namespaces/default/deployments
    fn parse_path(path: &str) -> Option<ParsedPath> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if parts.is_empty() {
            return None;
        }

        let (group, version_idx) = if parts[0] == "api" {
            (None, 1)
        } else if parts[0] == "apis" && parts.len() > 2 {
            (Some(parts[1].to_string()), 2)
        } else {
            return None;
        };

        if parts.len() < version_idx + 3 {
            return None;
        }

        let version = parts[version_idx].to_string();

        if parts[version_idx + 1] != "namespaces" {
            return None;
        }

        let namespace = parts[version_idx + 2].to_string();

        if parts.len() < version_idx + 4 {
            return None;
        }

        let resource = parts[version_idx + 3].to_string();
        let name = parts.get(version_idx + 4).map(|s| s.to_string());

        Some(ParsedPath {
            group,
            version,
            namespace,
            resource,
            name,
        })
    }

    /// Convert resource plural to singular kind (simplified)
    fn resource_to_kind(resource: &str) -> String {
        // Simple heuristic: remove trailing 's'
        if let Some(base) = resource.strip_suffix("ies") {
            format!("{}y", base)
        } else if resource.ends_with("ses")
            || resource.ends_with("xes")
            || resource.ends_with("zes")
        {
            resource[..resource.len() - 2].to_string()
        } else if let Some(base) = resource.strip_suffix('s') {
            base.to_string()
        } else {
            resource.to_string()
        }
    }

    async fn handle_request(
        &self,
        req: Request<KubeBody>,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        // Read the body
        let body_bytes = {
            use http_body_util::BodyExt;
            let body = req.into_body();
            let collected = body.collect().await?;
            collected.to_bytes()
        };

        // Parse the request based on HTTP method and path
        match method.as_str() {
            "GET" => self.handle_get(&path).await,
            "POST" => self.handle_post(&path, body_bytes).await,
            "PUT" => self.handle_put(&path, body_bytes).await,
            "PATCH" => self.handle_patch(&path, body_bytes).await,
            "DELETE" => self.handle_delete(&path).await,
            _ => Self::error_response(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed"),
        }
    }

    async fn handle_get(
        &self,
        path: &str,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;

        let gvr = GVR::new(
            parsed.group.clone().unwrap_or_default(),
            parsed.version.clone(),
            parsed.resource.clone(),
        );

        if let Some(name) = parsed.name {
            let is_status = path.ends_with("/status");

            let obj = if let Some(ref interceptors) = self.client.interceptors {
                if is_status {
                    if let Some(ref get_status_interceptor) = interceptors.get_status {
                        let ctx = interceptor::GetStatusContext {
                            client: &self.client,
                            namespace: &parsed.namespace,
                            name: &name,
                        };

                        match get_status_interceptor(ctx) {
                            Ok(Some(result)) => result,
                            Ok(None) => {
                                self.client.tracker().get(&gvr, &parsed.namespace, &name)?
                            }
                            Err(e) => {
                                return Self::error_response(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    &format!("Interceptor error: {}", e),
                                );
                            }
                        }
                    } else {
                        self.client.tracker().get(&gvr, &parsed.namespace, &name)?
                    }
                } else if let Some(ref get_interceptor) = interceptors.get {
                    let ctx = interceptor::GetContext {
                        client: &self.client,
                        namespace: &parsed.namespace,
                        name: &name,
                    };

                    match get_interceptor(ctx) {
                        Ok(Some(result)) => result,
                        Ok(None) => self.client.tracker().get(&gvr, &parsed.namespace, &name)?,
                        Err(e) => {
                            return Self::error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                &format!("Interceptor error: {}", e),
                            );
                        }
                    }
                } else {
                    self.client.tracker().get(&gvr, &parsed.namespace, &name)?
                }
            } else {
                self.client.tracker().get(&gvr, &parsed.namespace, &name)?
            };

            Self::success_response(obj)
        } else {
            let kind = Self::resource_to_kind(&parsed.resource);

            let objects = if let Some(ref interceptors) = self.client.interceptors {
                if let Some(ref list_interceptor) = interceptors.list {
                    let ctx = interceptor::ListContext {
                        client: &self.client,
                        namespace: Some(parsed.namespace.as_str()),
                        params: &ListParams::default(),
                    };

                    match list_interceptor(ctx) {
                        Ok(Some(result)) => result,
                        Ok(None) => self.client.tracker().list(&gvr, Some(&parsed.namespace))?,
                        Err(e) => {
                            return Self::error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                &format!("Interceptor error: {}", e),
                            );
                        }
                    }
                } else {
                    self.client.tracker().list(&gvr, Some(&parsed.namespace))?
                }
            } else {
                self.client.tracker().list(&gvr, Some(&parsed.namespace))?
            };

            let list = serde_json::json!({
                "kind": format!("{}List", kind),
                "apiVersion": if let Some(g) = parsed.group {
                    format!("{}/{}", g, parsed.version)
                } else {
                    parsed.version
                },
                "metadata": {
                    "resourceVersion": "1"
                },
                "items": objects
            });

            Self::success_response(list)
        }
    }

    async fn handle_post(
        &self,
        path: &str,
        body: Bytes,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;

        let mut obj: Value = serde_json::from_slice(&body)?;

        // Ensure apiVersion and kind are set
        let kind = Self::resource_to_kind(&parsed.resource);
        let api_version = if let Some(ref g) = parsed.group {
            format!("{}/{}", g, parsed.version)
        } else {
            parsed.version.clone()
        };

        if obj.get("apiVersion").is_none() {
            obj["apiVersion"] = serde_json::json!(api_version);
        }
        if obj.get("kind").is_none() {
            obj["kind"] = serde_json::json!(kind.clone());
        }

        let gvr = GVR::new(
            parsed.group.clone().unwrap_or_default(),
            parsed.version.clone(),
            parsed.resource,
        );
        let gvk = GVK::new(
            obj.get("apiVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .split('/')
                .next()
                .filter(|s| s.contains('.'))
                .map(String::from)
                .unwrap_or_default(),
            obj.get("apiVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .split('/')
                .next_back()
                .unwrap_or(&parsed.version)
                .to_string(),
            kind,
        );

        let created = if let Some(ref interceptors) = self.client.interceptors {
            if let Some(ref create_interceptor) = interceptors.create {
                let ctx = interceptor::CreateContext {
                    client: &self.client,
                    object: &obj,
                    namespace: &parsed.namespace,
                    params: &PostParams::default(),
                };

                match create_interceptor(ctx) {
                    Ok(Some(result)) => result,
                    Ok(None) => self
                        .client
                        .tracker()
                        .create(&gvr, &gvk, obj, &parsed.namespace)?,
                    Err(e) => {
                        return Self::error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            &format!("Interceptor error: {}", e),
                        );
                    }
                }
            } else {
                self.client
                    .tracker()
                    .create(&gvr, &gvk, obj, &parsed.namespace)?
            }
        } else {
            self.client
                .tracker()
                .create(&gvr, &gvk, obj, &parsed.namespace)?
        };

        Self::success_response(created)
    }

    async fn handle_put(
        &self,
        path: &str,
        body: Bytes,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;

        let name = parsed.name.as_ref().ok_or("Name required for PUT")?;
        let mut obj: Value = serde_json::from_slice(&body)?;

        let kind = Self::resource_to_kind(&parsed.resource);
        let api_version = if let Some(ref g) = parsed.group {
            format!("{}/{}", g, parsed.version)
        } else {
            parsed.version.clone()
        };

        if obj.get("apiVersion").is_none() {
            obj["apiVersion"] = serde_json::json!(api_version);
        }
        if obj.get("kind").is_none() {
            obj["kind"] = serde_json::json!(kind.clone());
        }

        let gvr = GVR::new(
            parsed.group.clone().unwrap_or_default(),
            parsed.version.clone(),
            parsed.resource,
        );
        let gvk = GVK::new(
            obj.get("apiVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .split('/')
                .next()
                .filter(|s| s.contains('.'))
                .map(String::from)
                .unwrap_or_default(),
            obj.get("apiVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .split('/')
                .next_back()
                .unwrap_or(&parsed.version)
                .to_string(),
            kind,
        );

        let is_status = path.ends_with("/status");

        let updated = if let Some(ref interceptors) = self.client.interceptors {
            if is_status {
                if let Some(ref replace_status_interceptor) = interceptors.replace_status {
                    let ctx = interceptor::ReplaceStatusContext {
                        client: &self.client,
                        object: &obj,
                        namespace: &parsed.namespace,
                        name,
                        params: &PostParams::default(),
                    };

                    match replace_status_interceptor(ctx) {
                        Ok(Some(result)) => result,
                        Ok(None) => self.client.tracker().update(
                            &gvr,
                            &gvk,
                            obj,
                            &parsed.namespace,
                            true,
                        )?,
                        Err(e) => {
                            return Self::error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                &format!("Interceptor error: {}", e),
                            );
                        }
                    }
                } else {
                    self.client
                        .tracker()
                        .update(&gvr, &gvk, obj, &parsed.namespace, true)?
                }
            } else if let Some(ref replace_interceptor) = interceptors.replace {
                let ctx = interceptor::ReplaceContext {
                    client: &self.client,
                    object: &obj,
                    namespace: &parsed.namespace,
                    name,
                    params: &PostParams::default(),
                };

                match replace_interceptor(ctx) {
                    Ok(Some(result)) => result,
                    Ok(None) => {
                        self.client
                            .tracker()
                            .update(&gvr, &gvk, obj, &parsed.namespace, false)?
                    }
                    Err(e) => {
                        return Self::error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            &format!("Interceptor error: {}", e),
                        );
                    }
                }
            } else {
                self.client
                    .tracker()
                    .update(&gvr, &gvk, obj, &parsed.namespace, false)?
            }
        } else {
            self.client
                .tracker()
                .update(&gvr, &gvk, obj, &parsed.namespace, is_status)?
        };

        Self::success_response(updated)
    }

    async fn handle_patch(
        &self,
        path: &str,
        body: Bytes,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;

        let name = parsed.name.ok_or("Name required for PATCH")?;
        let patch: Value = serde_json::from_slice(&body)?;

        let gvr = GVR::new(
            parsed.group.unwrap_or_default(),
            parsed.version,
            parsed.resource,
        );

        let is_status = path.ends_with("/status");

        let updated = if let Some(ref interceptors) = self.client.interceptors {
            if is_status {
                if let Some(ref patch_status_interceptor) = interceptors.patch_status {
                    let ctx = interceptor::PatchStatusContext {
                        client: &self.client,
                        patch: &patch,
                        namespace: &parsed.namespace,
                        name: &name,
                        params: &PatchParams::default(),
                    };

                    match patch_status_interceptor(ctx) {
                        Ok(Some(result)) => result,
                        Ok(None) => {
                            let mut existing =
                                self.client.tracker().get(&gvr, &parsed.namespace, &name)?;
                            json_patch::merge(&mut existing, &patch);
                            let gvk = extract_gvk(&existing)?;
                            self.client.tracker().update(
                                &gvr,
                                &gvk,
                                existing,
                                &parsed.namespace,
                                true,
                            )?
                        }
                        Err(e) => {
                            return Self::error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                &format!("Interceptor error: {}", e),
                            );
                        }
                    }
                } else {
                    // No interceptor - do default patch behavior
                    let mut existing = self.client.tracker().get(&gvr, &parsed.namespace, &name)?;
                    json_patch::merge(&mut existing, &patch);
                    let gvk = extract_gvk(&existing)?;
                    self.client
                        .tracker()
                        .update(&gvr, &gvk, existing, &parsed.namespace, true)?
                }
            } else if let Some(ref patch_interceptor) = interceptors.patch {
                let ctx = interceptor::PatchContext {
                    client: &self.client,
                    patch: &patch,
                    namespace: &parsed.namespace,
                    name: &name,
                    params: &PatchParams::default(),
                };

                match patch_interceptor(ctx) {
                    Ok(Some(result)) => result,
                    Ok(None) => {
                        let mut existing =
                            self.client.tracker().get(&gvr, &parsed.namespace, &name)?;
                        json_patch::merge(&mut existing, &patch);
                        let gvk = extract_gvk(&existing)?;
                        self.client.tracker().update(
                            &gvr,
                            &gvk,
                            existing,
                            &parsed.namespace,
                            false,
                        )?
                    }
                    Err(e) => {
                        return Self::error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            &format!("Interceptor error: {}", e),
                        );
                    }
                }
            } else {
                // No interceptor - do default patch behavior
                let mut existing = self.client.tracker().get(&gvr, &parsed.namespace, &name)?;
                json_patch::merge(&mut existing, &patch);
                let gvk = extract_gvk(&existing)?;
                self.client
                    .tracker()
                    .update(&gvr, &gvk, existing, &parsed.namespace, false)?
            }
        } else {
            let mut existing = self.client.tracker().get(&gvr, &parsed.namespace, &name)?;
            json_patch::merge(&mut existing, &patch);
            let gvk = extract_gvk(&existing)?;
            self.client
                .tracker()
                .update(&gvr, &gvk, existing, &parsed.namespace, is_status)?
        };

        Self::success_response(updated)
    }

    async fn handle_delete(
        &self,
        path: &str,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;

        let name = parsed.name.ok_or("Name required for DELETE")?;
        let gvr = GVR::new(
            parsed.group.unwrap_or_default(),
            parsed.version,
            parsed.resource,
        );

        let deleted = if let Some(ref interceptors) = self.client.interceptors {
            if let Some(ref delete_interceptor) = interceptors.delete {
                let ctx = interceptor::DeleteContext {
                    client: &self.client,
                    namespace: &parsed.namespace,
                    name: &name,
                };

                match delete_interceptor(ctx) {
                    Ok(Some(result)) => result,
                    Ok(None) => self
                        .client
                        .tracker()
                        .delete(&gvr, &parsed.namespace, &name)?,
                    Err(e) => {
                        return Self::error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            &format!("Interceptor error: {}", e),
                        );
                    }
                }
            } else {
                self.client
                    .tracker()
                    .delete(&gvr, &parsed.namespace, &name)?
            }
        } else {
            self.client
                .tracker()
                .delete(&gvr, &parsed.namespace, &name)?
        };

        Self::success_response(deleted)
    }

    fn error_response(
        status: StatusCode,
        message: &str,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let body = serde_json::json!({
            "kind": "Status",
            "apiVersion": "v1",
            "status": "Failure",
            "message": message,
            "code": status.as_u16()
        });

        Ok(Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap())
    }

    fn success_response(
        data: Value,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(data.to_string())))
            .unwrap())
    }
}

impl Service<Request<KubeBody>> for MockService {
    type Response = Response<Full<Bytes>>;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = BoxFuture<'static, std::result::Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<KubeBody>) -> Self::Future {
        let this = self.clone();
        async move { this.handle_request(req).await }.boxed()
    }
}
