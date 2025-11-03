//! Mock tower service that routes HTTP requests to the fake client

use crate::client::FakeClient;
use crate::client_utils::extract_gvk;
use crate::error::Error;
use crate::field_selectors::extract_preregistered_field_value;
use crate::interceptor;
use crate::tracker::GVR;
use bytes::Bytes;
use futures::future::{BoxFuture, FutureExt};
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use kube::api::{ListParams, PatchParams, PostParams};
use kube::client::Body as KubeBody;
use serde_json::Value;
use std::task::{Context, Poll};
use tower::Service;

/// Macro to handle crate::Error conversion to HTTP response
macro_rules! handle_error {
    ($result:expr) => {
        match $result {
            Ok(val) => val,
            Err(e) => return MockService::error_to_response(e),
        }
    };
}

/// Parsed Kubernetes API path information
struct ParsedPath {
    group: Option<String>,
    version: String,
    namespace: Option<String>,
    resource: String,
    name: Option<String>,
}

/// Patch types based on Content-Type header
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(clippy::enum_variant_names)]
enum PatchType {
    /// RFC 6902 JSON Patch - application/json-patch+json
    JsonPatch,
    /// RFC 7386 JSON Merge Patch - application/merge-patch+json
    MergePatch,
    /// Kubernetes Strategic Merge Patch - application/strategic-merge-patch+json
    StrategicMergePatch,
    /// Server-Side Apply - application/apply-patch+yaml
    ApplyPatch,
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
    /// - /api/v1/namespaces/default/pods (namespaced)
    /// - /api/v1/namespaces/default/pods/my-pod (namespaced with name)
    /// - /apis/apps/v1/namespaces/default/deployments (namespaced with group)
    /// - /api/v1/nodes (cluster-scoped)
    /// - /api/v1/nodes/node-1 (cluster-scoped with name)
    /// - /apis/rbac.authorization.k8s.io/v1/clusterroles (cluster-scoped with group)
    fn parse_path(path: &str) -> Option<ParsedPath> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if parts.is_empty() {
            return None;
        }

        // Determine API group and version index
        let (group, version_idx) = if parts[0] == "api" {
            (None, 1)
        } else if parts[0] == "apis" && parts.len() > 2 {
            (Some(parts[1].to_string()), 2)
        } else {
            return None;
        };

        // Need at least version + resource
        if parts.len() < version_idx + 2 {
            return None;
        }

        let version = parts[version_idx].to_string();

        // Check if this is a namespaced resource path
        if parts.get(version_idx + 1) == Some(&"namespaces") {
            // Namespaced resource: /api/v1/namespaces/{namespace}/{resource}[/{name}]
            if parts.len() < version_idx + 4 {
                return None;
            }

            let namespace = Some(parts[version_idx + 2].to_string());
            let resource = parts[version_idx + 3].to_string();
            let name = parts.get(version_idx + 4).map(|s| s.to_string());

            Some(ParsedPath {
                group,
                version,
                namespace,
                resource,
                name,
            })
        } else {
            // Cluster-scoped resource: /api/v1/{resource}[/{name}]
            let resource = parts[version_idx + 1].to_string();
            let name = parts.get(version_idx + 2).map(|s| s.to_string());

            Some(ParsedPath {
                group,
                version,
                namespace: None,
                resource,
                name,
            })
        }
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

    /// Parse query parameters from URL and create ListParams
    fn parse_list_params(query: Option<&str>) -> ListParams {
        let mut params = ListParams::default();

        if let Some(query_str) = query {
            for pair in query_str.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    // URL-decode the value
                    let decoded_value = urlencoding::decode(value).unwrap_or(std::borrow::Cow::Borrowed(value));

                    match key {
                        "labelSelector" => params.label_selector = Some(decoded_value.to_string()),
                        "fieldSelector" => params.field_selector = Some(decoded_value.to_string()),
                        "limit" => {
                            if let Ok(limit_val) = decoded_value.parse::<u32>() {
                                params.limit = Some(limit_val);
                            }
                        }
                        "continue" => params.continue_token = Some(decoded_value.to_string()),
                        "resourceVersion" => params.resource_version = Some(decoded_value.to_string()),
                        "timeoutSeconds" => {
                            if let Ok(timeout) = decoded_value.parse::<u32>() {
                                params.timeout = Some(timeout);
                            }
                        }
                        _ => {} // Ignore unknown parameters
                    }
                }
            }
        }

        params
    }

    /// Check if object matches label selector (simple equality-based matching)
    fn matches_label_selector(obj: &Value, selector: &str) -> bool {
        let labels = obj
            .get("metadata")
            .and_then(|m| m.get("labels"))
            .and_then(|l| l.as_object());

        if let Some(labels) = labels {
            for requirement in selector.split(',') {
                let requirement = requirement.trim();
                if let Some((key, value)) = requirement.split_once('=') {
                    let key = key.trim_end_matches('=');
                    let value = value.trim();
                    let label_value = labels.get(key).and_then(|v| v.as_str());
                    if label_value != Some(value) {
                        return false;
                    }
                } else {
                    // For now, only support key=value format
                    return false;
                }
            }
            true
        } else {
            // No labels, doesn't match any selector
            false
        }
    }

    /// Check if object matches field selector (uses pre-registered fields)
    fn matches_field_selector(obj: &Value, selector: &str) -> bool {
        // Extract kind from object to determine which fields are available
        let kind = obj
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("");

        for requirement in selector.split(',') {
            let requirement = requirement.trim();
            if let Some((field, expected_value)) = requirement.split_once('=') {
                let field = field.trim_end_matches('=');
                let expected_value = expected_value.trim();

                // Try to extract the field value using pre-registered fields
                let values = extract_preregistered_field_value(obj, field, kind);

                // Check if any of the values match the expected value
                if !values.map_or(false, |v| v.iter().any(|val| val == expected_value)) {
                    return false;
                }
            }
        }
        true
    }

    /// Determine patch type from Content-Type header
    fn determine_patch_type(content_type: Option<&str>) -> PatchType {
        match content_type {
            Some(ct) if ct.contains("application/json-patch+json") => PatchType::JsonPatch,
            Some(ct) if ct.contains("application/merge-patch+json") => PatchType::MergePatch,
            Some(ct) if ct.contains("application/strategic-merge-patch+json") => {
                PatchType::StrategicMergePatch
            }
            Some(ct) if ct.contains("application/apply-patch+yaml") => PatchType::ApplyPatch,
            // Default to Strategic Merge Patch for Kubernetes compatibility
            _ => PatchType::StrategicMergePatch,
        }
    }

    /// Apply patch to existing object based on patch type
    fn apply_patch(
        existing: &mut Value,
        patch: &Value,
        patch_type: PatchType,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match patch_type {
            PatchType::JsonPatch => {
                // RFC 6902 JSON Patch - array of operations
                let patch_doc: json_patch::Patch = serde_json::from_value(patch.clone())?;
                json_patch::patch(existing, &patch_doc)?;
            }
            PatchType::MergePatch => {
                // RFC 7386 JSON Merge Patch
                json_patch::merge(existing, patch);
            }
            PatchType::StrategicMergePatch => {
                // For now, treat Strategic Merge Patch like Merge Patch
                // Full strategic merge would require schema knowledge
                json_patch::merge(existing, patch);
            }
            PatchType::ApplyPatch => {
                // For now, treat Server-Side Apply like Merge Patch
                // Full SSA would require field management and ownership tracking
                json_patch::merge(existing, patch);
            }
        }
        Ok(())
    }

    async fn handle_request(
        &self,
        req: Request<KubeBody>,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let method = req.method().clone();
        let path = req.uri().path().to_string();
        let query = req.uri().query().map(|s| s.to_string());
        let content_type = req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        // Read the body
        let body_bytes = {
            use http_body_util::BodyExt;
            let body = req.into_body();
            let collected = body.collect().await?;
            collected.to_bytes()
        };

        // Parse the request based on HTTP method and path
        match method.as_str() {
            "GET" => self.handle_get(&path, query.as_deref()).await,
            "POST" => self.handle_post(&path, body_bytes).await,
            "PUT" => self.handle_put(&path, body_bytes).await,
            "PATCH" => {
                self.handle_patch(&path, body_bytes, content_type.as_deref())
                    .await
            }
            "DELETE" => self.handle_delete(&path).await,
            _ => Self::error_response(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed"),
        }
    }

    async fn handle_get(
        &self,
        path: &str,
        query: Option<&str>,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;

        let gvr = GVR::new(
            parsed.group.clone().unwrap_or_default(),
            parsed.version.clone(),
            parsed.resource.clone(),
        );

        // Convert Option<String> to &str, using empty string for cluster-scoped resources
        let namespace = parsed.namespace.as_deref().unwrap_or("");

        if let Some(name) = parsed.name {
            let is_status = path.ends_with("/status");

            let obj = if let Some(ref interceptors) = self.client.interceptors {
                if is_status {
                    if let Some(ref get_status_interceptor) = interceptors.get_status {
                        let ctx = interceptor::GetStatusContext {
                            client: &self.client,
                            namespace,
                            name: &name,
                        };

                        match get_status_interceptor(ctx) {
                            Ok(Some(result)) => result,
                            Ok(None) => {
                                handle_error!(self.client.tracker().get(&gvr, namespace, &name))
                            }
                            Err(e) => {
                                return Self::error_response(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    &format!("Interceptor error: {}", e),
                                );
                            }
                        }
                    } else {
                        handle_error!(self.client.tracker().get(&gvr, namespace, &name))
                    }
                } else if let Some(ref get_interceptor) = interceptors.get {
                    let ctx = interceptor::GetContext {
                        client: &self.client,
                        namespace,
                        name: &name,
                    };

                    match get_interceptor(ctx) {
                        Ok(Some(result)) => result,
                        Ok(None) => {
                            handle_error!(self.client.tracker().get(&gvr, namespace, &name))
                        }
                        Err(e) => {
                            return Self::error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                &format!("Interceptor error: {}", e),
                            );
                        }
                    }
                } else {
                    handle_error!(self.client.tracker().get(&gvr, namespace, &name))
                }
            } else {
                handle_error!(self.client.tracker().get(&gvr, namespace, &name))
            };

            Self::success_response(obj)
        } else {
            let kind = Self::resource_to_kind(&parsed.resource);
            let list_params = Self::parse_list_params(query);

            let mut objects = if let Some(ref interceptors) = self.client.interceptors {
                if let Some(ref list_interceptor) = interceptors.list {
                    let ctx = interceptor::ListContext {
                        client: &self.client,
                        namespace: parsed.namespace.as_deref(),
                        params: &list_params,
                    };

                    match list_interceptor(ctx) {
                        Ok(Some(result)) => result,
                        Ok(None) => handle_error!(self
                            .client
                            .tracker()
                            .list(&gvr, parsed.namespace.as_deref())),
                        Err(e) => {
                            return Self::error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                &format!("Interceptor error: {}", e),
                            );
                        }
                    }
                } else {
                    handle_error!(self
                        .client
                        .tracker()
                        .list(&gvr, parsed.namespace.as_deref()))
                }
            } else {
                handle_error!(self
                    .client
                    .tracker()
                    .list(&gvr, parsed.namespace.as_deref()))
            };

            // Apply label selector filtering if specified
            if let Some(label_selector) = &list_params.label_selector {
                objects.retain(|obj| Self::matches_label_selector(obj, label_selector));
            }

            // Apply field selector filtering if specified
            if let Some(field_selector) = &list_params.field_selector {
                objects.retain(|obj| Self::matches_field_selector(obj, field_selector));
            }

            // Apply limit if specified
            if let Some(limit) = list_params.limit {
                objects.truncate(limit as usize);
            }

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

        // Convert Option<String> to &str, using empty string for cluster-scoped resources
        let namespace = parsed.namespace.as_deref().unwrap_or("");

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
            obj["kind"] = serde_json::json!(kind);
        }

        let gvr = GVR::new(
            parsed.group.clone().unwrap_or_default(),
            parsed.version.clone(),
            parsed.resource,
        );
        let gvk = extract_gvk(&obj)?;

        let created = if let Some(ref interceptors) = self.client.interceptors {
            if let Some(ref create_interceptor) = interceptors.create {
                let ctx = interceptor::CreateContext {
                    client: &self.client,
                    object: &obj,
                    namespace,
                    params: &PostParams::default(),
                };

                match create_interceptor(ctx) {
                    Ok(Some(result)) => result,
                    Ok(None) => self.client.tracker().create(&gvr, &gvk, obj, namespace)?,
                    Err(e) => {
                        return Self::error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            &format!("Interceptor error: {}", e),
                        );
                    }
                }
            } else {
                self.client.tracker().create(&gvr, &gvk, obj, namespace)?
            }
        } else {
            self.client.tracker().create(&gvr, &gvk, obj, namespace)?
        };

        Self::success_response(created)
    }

    async fn handle_put(
        &self,
        path: &str,
        body: Bytes,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;

        // Convert Option<String> to &str, using empty string for cluster-scoped resources
        let namespace = parsed.namespace.as_deref().unwrap_or("");

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
            obj["kind"] = serde_json::json!(kind);
        }

        let gvr = GVR::new(
            parsed.group.clone().unwrap_or_default(),
            parsed.version.clone(),
            parsed.resource,
        );
        let gvk = extract_gvk(&obj)?;

        let is_status = path.ends_with("/status");

        let updated = if let Some(ref interceptors) = self.client.interceptors {
            if is_status {
                if let Some(ref replace_status_interceptor) = interceptors.replace_status {
                    let ctx = interceptor::ReplaceStatusContext {
                        client: &self.client,
                        object: &obj,
                        namespace,
                        name,
                        params: &PostParams::default(),
                    };

                    match replace_status_interceptor(ctx) {
                        Ok(Some(result)) => result,
                        Ok(None) => self
                            .client
                            .tracker()
                            .update(&gvr, &gvk, obj, namespace, true)?,
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
                        .update(&gvr, &gvk, obj, namespace, true)?
                }
            } else if let Some(ref replace_interceptor) = interceptors.replace {
                let ctx = interceptor::ReplaceContext {
                    client: &self.client,
                    object: &obj,
                    namespace,
                    name,
                    params: &PostParams::default(),
                };

                match replace_interceptor(ctx) {
                    Ok(Some(result)) => result,
                    Ok(None) => self
                        .client
                        .tracker()
                        .update(&gvr, &gvk, obj, namespace, false)?,
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
                    .update(&gvr, &gvk, obj, namespace, false)?
            }
        } else {
            self.client
                .tracker()
                .update(&gvr, &gvk, obj, namespace, is_status)?
        };

        Self::success_response(updated)
    }

    async fn handle_patch(
        &self,
        path: &str,
        body: Bytes,
        content_type: Option<&str>,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;

        // Convert Option<String> to &str, using empty string for cluster-scoped resources
        let namespace = parsed.namespace.as_deref().unwrap_or("");

        let name = parsed.name.ok_or("Name required for PATCH")?;
        let patch: Value = serde_json::from_slice(&body)?;

        // Determine patch type from Content-Type header
        let patch_type = Self::determine_patch_type(content_type);

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
                        namespace,
                        name: &name,
                        params: &PatchParams::default(),
                    };

                    match patch_status_interceptor(ctx) {
                        Ok(Some(result)) => result,
                        Ok(None) => {
                            let mut existing = self.client.tracker().get(&gvr, namespace, &name)?;
                            Self::apply_patch(&mut existing, &patch, patch_type)?;
                            let gvk = extract_gvk(&existing)?;
                            self.client
                                .tracker()
                                .update(&gvr, &gvk, existing, namespace, true)?
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
                    let mut existing = self.client.tracker().get(&gvr, namespace, &name)?;
                    Self::apply_patch(&mut existing, &patch, patch_type)?;
                    let gvk = extract_gvk(&existing)?;
                    self.client
                        .tracker()
                        .update(&gvr, &gvk, existing, namespace, true)?
                }
            } else if let Some(ref patch_interceptor) = interceptors.patch {
                let ctx = interceptor::PatchContext {
                    client: &self.client,
                    patch: &patch,
                    namespace,
                    name: &name,
                    params: &PatchParams::default(),
                };

                match patch_interceptor(ctx) {
                    Ok(Some(result)) => result,
                    Ok(None) => {
                        let mut existing = self.client.tracker().get(&gvr, namespace, &name)?;
                        Self::apply_patch(&mut existing, &patch, patch_type)?;
                        let gvk = extract_gvk(&existing)?;
                        self.client
                            .tracker()
                            .update(&gvr, &gvk, existing, namespace, false)?
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
                let mut existing = self.client.tracker().get(&gvr, namespace, &name)?;
                Self::apply_patch(&mut existing, &patch, patch_type)?;
                let gvk = extract_gvk(&existing)?;
                self.client
                    .tracker()
                    .update(&gvr, &gvk, existing, namespace, false)?
            }
        } else {
            let mut existing = self.client.tracker().get(&gvr, namespace, &name)?;
            Self::apply_patch(&mut existing, &patch, patch_type)?;
            let gvk = extract_gvk(&existing)?;
            self.client
                .tracker()
                .update(&gvr, &gvk, existing, namespace, is_status)?
        };

        Self::success_response(updated)
    }

    async fn handle_delete(
        &self,
        path: &str,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;

        // Convert Option<String> to &str, using empty string for cluster-scoped resources
        let namespace = parsed.namespace.as_deref().unwrap_or("");

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
                    namespace,
                    name: &name,
                };

                match delete_interceptor(ctx) {
                    Ok(Some(result)) => result,
                    Ok(None) => self.client.tracker().delete(&gvr, namespace, &name)?,
                    Err(e) => {
                        return Self::error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            &format!("Interceptor error: {}", e),
                        );
                    }
                }
            } else {
                self.client.tracker().delete(&gvr, namespace, &name)?
            }
        } else {
            self.client.tracker().delete(&gvr, namespace, &name)?
        };

        Self::success_response(deleted)
    }

    /// Convert crate::Error to proper HTTP response matching Kubernetes API format
    fn error_to_response(
        err: Error,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        // Convert to kube::Error to get proper ErrorResponse
        let kube_err = err.into_kube_err();

        // Extract ErrorResponse from kube::Error::Api
        if let kube::Error::Api(error_response) = kube_err {
            let status_code = StatusCode::from_u16(error_response.code)
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            // Return Status object matching Kubernetes API format
            let body = serde_json::json!({
                "kind": "Status",
                "apiVersion": "v1",
                "status": error_response.status,
                "message": error_response.message,
                "reason": error_response.reason,
                "code": error_response.code
            });

            Ok(Response::builder()
                .status(status_code)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(body.to_string())))
                .unwrap())
        } else {
            // Fallback for non-Api errors
            Self::error_response(StatusCode::INTERNAL_SERVER_ERROR, &kube_err.to_string())
        }
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
