//! Mock tower service that routes HTTP requests to the fake client

use crate::client::FakeClient;
use crate::client_utils::extract_gvk;
use crate::discovery::Discovery;
use crate::error::Error;
use crate::field_selectors::extract_preregistered_field_value;
use crate::interceptor;
use crate::label_selector;
use crate::tracker::GVR;
use bytes::Bytes;
use futures::future::{BoxFuture, FutureExt};
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use kube::api::{ListParams, PatchParams, PostParams};
use kube::client::Body as KubeBody;
use serde_json::Value;
use std::collections::BTreeMap;
use std::task::{Context, Poll};
use tower::Service;

/// Content type constants
const CONTENT_TYPE_JSON: &str = "application/json";
const CONTENT_TYPE_JSON_PATCH: &str = "application/json-patch+json";
const CONTENT_TYPE_MERGE_PATCH: &str = "application/merge-patch+json";
const CONTENT_TYPE_STRATEGIC_MERGE: &str = "application/strategic-merge-patch+json";
const CONTENT_TYPE_APPLY_PATCH: &str = "application/apply-patch+yaml";

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
#[derive(Debug, Clone)]
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

            Some(ParsedPath {
                group,
                version,
                namespace: Some(parts[version_idx + 2].to_string()),
                resource: parts[version_idx + 3].to_string(),
                name: parts.get(version_idx + 4).map(|s| s.to_string()),
            })
        } else {
            // Cluster-scoped resource: /api/v1/{resource}[/{name}]
            Some(ParsedPath {
                group,
                version,
                namespace: None,
                resource: parts[version_idx + 1].to_string(),
                name: parts.get(version_idx + 2).map(|s| s.to_string()),
            })
        }
    }

    /// Convert resource plural to Kind using discovery + registry
    fn resource_to_kind(
        &self,
        group: &str,
        version: &str,
        resource: &str,
    ) -> Result<String, Error> {
        Discovery::plural_to_kind_with_registry(group, version, resource, &self.client.registry)
            .map(|k| k.into_owned())
            .ok_or_else(|| Error::ResourceNotRegistered {
                group: group.to_string(),
                version: version.to_string(),
                resource: resource.to_string(),
            })
    }

    /// Extract namespace from parsed path, defaulting to empty string for cluster-scoped
    fn extract_namespace(parsed: &ParsedPath) -> String {
        parsed.namespace.as_deref().unwrap_or("").to_string()
    }

    /// Build API version string from group and version
    fn build_api_version(group: &Option<String>, version: &str) -> String {
        match group {
            Some(g) => format!("{g}/{version}"),
            None => version.to_string(),
        }
    }

    /// Extract object name from metadata
    fn extract_object_name(obj: &Value) -> Option<String> {
        obj.get("metadata")
            .and_then(|m| m.get("name"))
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
    }

    /// Parse query parameters from URL and create ListParams
    fn parse_list_params(query: Option<&str>) -> ListParams {
        let mut params = ListParams::default();

        if let Some(query_str) = query {
            for pair in query_str.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    let decoded_value =
                        urlencoding::decode(value).unwrap_or(std::borrow::Cow::Borrowed(value));

                    match key {
                        "labelSelector" => params.label_selector = Some(decoded_value.to_string()),
                        "fieldSelector" => params.field_selector = Some(decoded_value.to_string()),
                        "limit" => {
                            if let Ok(limit_val) = decoded_value.parse::<u32>() {
                                params.limit = Some(limit_val);
                            }
                        }
                        "continue" => params.continue_token = Some(decoded_value.to_string()),
                        "resourceVersion" => {
                            params.resource_version = Some(decoded_value.to_string())
                        }
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

    /// Check if object matches label selector
    fn matches_label_selector(obj: &Value, selector: &str) -> bool {
        let labels_obj = obj
            .get("metadata")
            .and_then(|m| m.get("labels"))
            .and_then(|l| l.as_object());

        let labels: BTreeMap<String, String> = labels_obj
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        label_selector::matches_label_selector(&labels, selector).unwrap_or(false)
    }

    /// Check if object matches field selector (uses pre-registered fields)
    fn matches_field_selector(obj: &Value, selector: &str) -> bool {
        let kind = obj.get("kind").and_then(|k| k.as_str()).unwrap_or("");

        for requirement in selector.split(',') {
            let requirement = requirement.trim();
            if let Some((field, expected_value)) = requirement.split_once('=') {
                let field = field.trim_end_matches('=');
                let expected_value = expected_value.trim();

                let values = extract_preregistered_field_value(obj, field, kind);

                if !values.is_some_and(|v| v.iter().any(|val| val == expected_value)) {
                    return false;
                }
            }
        }
        true
    }

    /// Determine patch type from Content-Type header
    fn determine_patch_type(content_type: Option<&str>) -> PatchType {
        match content_type {
            Some(ct) if ct.contains(CONTENT_TYPE_JSON_PATCH) => PatchType::JsonPatch,
            Some(ct) if ct.contains(CONTENT_TYPE_MERGE_PATCH) => PatchType::MergePatch,
            Some(ct) if ct.contains(CONTENT_TYPE_STRATEGIC_MERGE) => PatchType::StrategicMergePatch,
            Some(ct) if ct.contains(CONTENT_TYPE_APPLY_PATCH) => PatchType::ApplyPatch,
            _ => PatchType::StrategicMergePatch, // Default for Kubernetes compatibility
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
                let patch_doc: json_patch::Patch = serde_json::from_value(patch.clone())?;
                json_patch::patch(existing, &patch_doc)?;
            }
            PatchType::MergePatch | PatchType::StrategicMergePatch | PatchType::ApplyPatch => {
                // For now, treat all merge-style patches the same
                // Full strategic merge would require schema knowledge
                json_patch::merge(existing, patch);
            }
        }
        Ok(())
    }

    /// Execute interceptor or default action for GET operations
    fn execute_get_with_interceptor(
        &self,
        gvr: &GVR,
        namespace: &str,
        name: &str,
        is_status: bool,
    ) -> std::result::Result<Value, Error> {
        if let Some(ref interceptors) = self.client.interceptors {
            if is_status {
                if let Some(ref get_status_interceptor) = interceptors.get_status {
                    let ctx = interceptor::GetStatusContext {
                        client: &self.client,
                        namespace,
                        name,
                    };
                    return match get_status_interceptor(ctx) {
                        Ok(Some(result)) => Ok(result),
                        Ok(None) => self.client.tracker().get(gvr, namespace, name),
                        Err(e) => Err(e),
                    };
                }
            } else if let Some(ref get_interceptor) = interceptors.get {
                let ctx = interceptor::GetContext {
                    client: &self.client,
                    namespace,
                    name,
                };
                return match get_interceptor(ctx) {
                    Ok(Some(result)) => Ok(result),
                    Ok(None) => self.client.tracker().get(gvr, namespace, name),
                    Err(e) => Err(e),
                };
            }
        }
        self.client.tracker().get(gvr, namespace, name)
    }

    /// Execute interceptor or default action for LIST operations
    fn execute_list_with_interceptor(
        &self,
        gvr: &GVR,
        namespace: Option<&str>,
        params: &ListParams,
    ) -> std::result::Result<Vec<Value>, Error> {
        if let Some(ref interceptors) = self.client.interceptors {
            if let Some(ref list_interceptor) = interceptors.list {
                let ctx = interceptor::ListContext {
                    client: &self.client,
                    namespace,
                    params,
                };
                return match list_interceptor(ctx) {
                    Ok(Some(result)) => Ok(result),
                    Ok(None) => self.client.tracker().list(gvr, namespace),
                    Err(e) => Err(e),
                };
            }
        }
        self.client.tracker().list(gvr, namespace)
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

        // Route based on HTTP method
        match method.as_str() {
            "GET" => self.handle_get(&path, query.as_deref()).await,
            "POST" => self.handle_post(&path, body_bytes).await,
            "PUT" => self.handle_put(&path, body_bytes).await,
            "PATCH" => {
                self.handle_patch(&path, body_bytes, content_type.as_deref())
                    .await
            }
            "DELETE" => self.handle_delete(&path, query.as_deref()).await,
            _ => Self::error_response(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed"),
        }
    }

    async fn handle_get(
        &self,
        path: &str,
        query: Option<&str>,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;
        let namespace = Self::extract_namespace(&parsed);
        let kind = handle_error!(self.resource_to_kind(
            &parsed.group.clone().unwrap_or_default(),
            &parsed.version,
            &parsed.resource
        ));

        let gvr = GVR::new(
            parsed.group.clone().unwrap_or_default(),
            parsed.version.clone(),
            parsed.resource.clone(),
        );

        let gvk = crate::tracker::GVK::new(
            parsed.group.clone().unwrap_or_default(),
            parsed.version.clone(),
            &kind,
        );

        if let Some(name) = parsed.name {
            // GET single object
            handle_error!(self.client.validate_verb(&gvk, "get"));
            let is_status = path.ends_with("/status");

            let obj = handle_error!(
                self.execute_get_with_interceptor(&gvr, &namespace, &name, is_status)
            );
            Self::success_response(obj)
        } else {
            // LIST objects
            handle_error!(self.client.validate_verb(&gvk, "list"));

            let list_params = Self::parse_list_params(query);
            let mut objects = handle_error!(self.execute_list_with_interceptor(
                &gvr,
                parsed.namespace.as_deref(),
                &list_params
            ));

            // Apply selectors
            if let Some(label_selector) = &list_params.label_selector {
                objects.retain(|obj| Self::matches_label_selector(obj, label_selector));
            }

            if let Some(field_selector) = &list_params.field_selector {
                objects.retain(|obj| Self::matches_field_selector(obj, field_selector));
            }

            // Apply limit
            if let Some(limit) = list_params.limit {
                objects.truncate(limit as usize);
            }

            let list = serde_json::json!({
                "kind": format!("{kind}List"),
                "apiVersion": Self::build_api_version(&parsed.group, &parsed.version),
                "metadata": { "resourceVersion": "1" },
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
        let namespace = Self::extract_namespace(&parsed);

        let mut obj: Value = serde_json::from_slice(&body)?;

        let kind = handle_error!(self.resource_to_kind(
            &parsed.group.clone().unwrap_or_default(),
            &parsed.version,
            &parsed.resource
        ));

        // Ensure apiVersion and kind are set
        let api_version = Self::build_api_version(&parsed.group, &parsed.version);
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

        handle_error!(self.client.validate_verb(&gvk, "create"));

        let created = if let Some(ref interceptors) = self.client.interceptors {
            if let Some(ref create_interceptor) = interceptors.create {
                let ctx = interceptor::CreateContext {
                    client: &self.client,
                    object: &obj,
                    namespace: &namespace,
                    params: &PostParams::default(),
                };

                match create_interceptor(ctx) {
                    Ok(Some(result)) => result,
                    Ok(None) => {
                        handle_error!(self.client.tracker().create(&gvr, &gvk, obj, &namespace))
                    }
                    Err(e) => return Self::error_to_response(e),
                }
            } else {
                handle_error!(self.client.tracker().create(&gvr, &gvk, obj, &namespace))
            }
        } else {
            handle_error!(self.client.tracker().create(&gvr, &gvk, obj, &namespace))
        };

        Self::success_response_with_status(created, StatusCode::CREATED)
    }

    async fn handle_put(
        &self,
        path: &str,
        body: Bytes,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;
        let namespace = Self::extract_namespace(&parsed);
        let name = parsed.name.as_ref().ok_or("Name required for PUT")?;

        let mut obj: Value = serde_json::from_slice(&body)?;

        let kind = handle_error!(self.resource_to_kind(
            &parsed.group.clone().unwrap_or_default(),
            &parsed.version,
            &parsed.resource
        ));

        let api_version = Self::build_api_version(&parsed.group, &parsed.version);
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

        handle_error!(self.client.validate_verb(&gvk, "update"));

        let updated = if let Some(ref interceptors) = self.client.interceptors {
            if is_status {
                if let Some(ref replace_status_interceptor) = interceptors.replace_status {
                    let ctx = interceptor::ReplaceStatusContext {
                        client: &self.client,
                        object: &obj,
                        namespace: &namespace,
                        name,
                        params: &PostParams::default(),
                    };

                    match replace_status_interceptor(ctx) {
                        Ok(Some(result)) => result,
                        Ok(None) => handle_error!(self
                            .client
                            .tracker()
                            .update(&gvr, &gvk, obj, &namespace, true)),
                        Err(e) => return Self::error_to_response(e),
                    }
                } else {
                    handle_error!(self
                        .client
                        .tracker()
                        .update(&gvr, &gvk, obj, &namespace, true))
                }
            } else if let Some(ref replace_interceptor) = interceptors.replace {
                let ctx = interceptor::ReplaceContext {
                    client: &self.client,
                    object: &obj,
                    namespace: &namespace,
                    name,
                    params: &PostParams::default(),
                };

                match replace_interceptor(ctx) {
                    Ok(Some(result)) => result,
                    Ok(None) => handle_error!(self
                        .client
                        .tracker()
                        .update(&gvr, &gvk, obj, &namespace, false)),
                    Err(e) => return Self::error_to_response(e),
                }
            } else {
                handle_error!(self
                    .client
                    .tracker()
                    .update(&gvr, &gvk, obj, &namespace, false))
            }
        } else {
            handle_error!(self
                .client
                .tracker()
                .update(&gvr, &gvk, obj, &namespace, is_status))
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
        let namespace = Self::extract_namespace(&parsed);
        let name = parsed.name.ok_or("Name required for PATCH")?;

        let patch: Value = serde_json::from_slice(&body)?;
        let patch_type = Self::determine_patch_type(content_type);

        let gvr = GVR::new(
            parsed.group.clone().unwrap_or_default(),
            parsed.version.clone(),
            parsed.resource.clone(),
        );

        let kind = handle_error!(self.resource_to_kind(
            &parsed.group.clone().unwrap_or_default(),
            &parsed.version,
            &parsed.resource
        ));
        let gvk = crate::tracker::GVK::new(parsed.group.unwrap_or_default(), parsed.version, &kind);
        let is_status = path.ends_with("/status");

        handle_error!(self.client.validate_verb(&gvk, "patch"));

        let updated = if let Some(ref interceptors) = self.client.interceptors {
            if is_status {
                if let Some(ref patch_status_interceptor) = interceptors.patch_status {
                    let ctx = interceptor::PatchStatusContext {
                        client: &self.client,
                        patch: &patch,
                        namespace: &namespace,
                        name: &name,
                        params: &PatchParams::default(),
                    };

                    match patch_status_interceptor(ctx) {
                        Ok(Some(result)) => result,
                        Ok(None) => {
                            let mut existing =
                                handle_error!(self.client.tracker().get(&gvr, &namespace, &name));
                            Self::apply_patch(&mut existing, &patch, patch_type)?;
                            let gvk = extract_gvk(&existing)?;
                            handle_error!(self
                                .client
                                .tracker()
                                .update(&gvr, &gvk, existing, &namespace, true))
                        }
                        Err(e) => return Self::error_to_response(e),
                    }
                } else {
                    let mut existing =
                        handle_error!(self.client.tracker().get(&gvr, &namespace, &name));
                    Self::apply_patch(&mut existing, &patch, patch_type)?;
                    let gvk = extract_gvk(&existing)?;
                    handle_error!(self
                        .client
                        .tracker()
                        .update(&gvr, &gvk, existing, &namespace, true))
                }
            } else if let Some(ref patch_interceptor) = interceptors.patch {
                let ctx = interceptor::PatchContext {
                    client: &self.client,
                    patch: &patch,
                    namespace: &namespace,
                    name: &name,
                    params: &PatchParams::default(),
                };

                match patch_interceptor(ctx) {
                    Ok(Some(result)) => result,
                    Ok(None) => {
                        let mut existing =
                            handle_error!(self.client.tracker().get(&gvr, &namespace, &name));
                        Self::apply_patch(&mut existing, &patch, patch_type)?;
                        let gvk = extract_gvk(&existing)?;
                        handle_error!(self
                            .client
                            .tracker()
                            .update(&gvr, &gvk, existing, &namespace, false))
                    }
                    Err(e) => return Self::error_to_response(e),
                }
            } else {
                let mut existing =
                    handle_error!(self.client.tracker().get(&gvr, &namespace, &name));
                Self::apply_patch(&mut existing, &patch, patch_type)?;
                let gvk = extract_gvk(&existing)?;
                handle_error!(self
                    .client
                    .tracker()
                    .update(&gvr, &gvk, existing, &namespace, false))
            }
        } else {
            let mut existing = handle_error!(self.client.tracker().get(&gvr, &namespace, &name));
            Self::apply_patch(&mut existing, &patch, patch_type)?;
            let gvk = extract_gvk(&existing)?;
            handle_error!(self
                .client
                .tracker()
                .update(&gvr, &gvk, existing, &namespace, is_status))
        };

        Self::success_response(updated)
    }

    async fn handle_delete(
        &self,
        path: &str,
        query: Option<&str>,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let parsed = Self::parse_path(path).ok_or("Invalid path")?;
        let namespace = Self::extract_namespace(&parsed);

        let gvr = GVR::new(
            parsed.group.clone().unwrap_or_default(),
            parsed.version.clone(),
            parsed.resource.clone(),
        );

        let kind = handle_error!(self.resource_to_kind(
            &parsed.group.clone().unwrap_or_default(),
            &parsed.version,
            &parsed.resource
        ));
        let gvk = crate::tracker::GVK::new(
            parsed.group.clone().unwrap_or_default(),
            parsed.version.clone(),
            &kind,
        );

        handle_error!(self.client.validate_verb(&gvk, "delete"));

        if let Some(name) = parsed.name {
            // Single object deletion
            let deleted = if let Some(ref interceptors) = self.client.interceptors {
                if let Some(ref delete_interceptor) = interceptors.delete {
                    let ctx = interceptor::DeleteContext {
                        client: &self.client,
                        namespace: &namespace,
                        name: &name,
                    };

                    match delete_interceptor(ctx) {
                        Ok(Some(result)) => result,
                        Ok(None) => {
                            handle_error!(self.client.tracker().delete(&gvr, &namespace, &name))
                        }
                        Err(e) => return Self::error_to_response(e),
                    }
                } else {
                    handle_error!(self.client.tracker().delete(&gvr, &namespace, &name))
                }
            } else {
                handle_error!(self.client.tracker().delete(&gvr, &namespace, &name))
            };

            Self::success_response(deleted)
        } else {
            // Collection deletion
            let list_params = Self::parse_list_params(query);
            let mut objects = handle_error!(self
                .client
                .tracker()
                .list(&gvr, parsed.namespace.as_deref()));

            // Apply selectors
            if let Some(label_selector) = &list_params.label_selector {
                objects.retain(|obj| Self::matches_label_selector(obj, label_selector));
            }

            if let Some(field_selector) = &list_params.field_selector {
                objects.retain(|obj| Self::matches_field_selector(obj, field_selector));
            }

            // Delete each matching object
            let deleted_count = objects
                .iter()
                .filter_map(Self::extract_object_name)
                .filter(|obj_name| {
                    self.client
                        .tracker()
                        .delete(&gvr, &namespace, obj_name)
                        .is_ok()
                })
                .count();

            let status_response = serde_json::json!({
                "kind": "Status",
                "apiVersion": "v1",
                "status": "Success",
                "details": {
                    "kind": kind,
                    "group": parsed.group.unwrap_or_default(),
                    "deleted": deleted_count
                }
            });

            Self::success_response(status_response)
        }
    }

    /// Convert crate::Error to proper HTTP response matching Kubernetes API format
    fn error_to_response(
        err: Error,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        let kube_err = err.into_kube_err();

        if let kube::Error::Api(error_response) = kube_err {
            let status_code = StatusCode::from_u16(error_response.code)
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

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
                .header("Content-Type", CONTENT_TYPE_JSON)
                .body(Full::new(Bytes::from(body.to_string())))
                .expect("Failed to build response"))
        } else {
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
            .header("Content-Type", CONTENT_TYPE_JSON)
            .body(Full::new(Bytes::from(body.to_string())))
            .expect("Failed to build response"))
    }

    fn success_response(
        data: Value,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        Self::success_response_with_status(data, StatusCode::OK)
    }

    fn success_response_with_status(
        data: Value,
        status: StatusCode,
    ) -> std::result::Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Response::builder()
            .status(status)
            .header("Content-Type", CONTENT_TYPE_JSON)
            .body(Full::new(Bytes::from(data.to_string())))
            .expect("Failed to build response"))
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
