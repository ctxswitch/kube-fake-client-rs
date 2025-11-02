//! Interceptors for customizing client behavior during testing

use crate::client::FakeClient;
use crate::Result;
use kube::api::{ListParams, PatchParams, PostParams};
use serde_json::Value;
use std::sync::Arc;

/// Interceptor functions for client operations
///
/// Return `Ok(Some(value))` to override, `Ok(None)` to continue, or `Err(e)` to inject an error.
#[derive(Default)]
pub struct Funcs {
    /// Intercept Create operations
    pub create: Option<CreateInterceptor>,
    /// Intercept Get operations
    pub get: Option<GetInterceptor>,
    /// Intercept Update operations (PATCH-based updates)
    pub update: Option<UpdateInterceptor>,
    /// Intercept Replace operations (PUT - full replacement)
    pub replace: Option<ReplaceInterceptor>,
    /// Intercept Delete operations
    pub delete: Option<DeleteInterceptor>,
    /// Intercept Delete Collection operations
    pub delete_collection: Option<DeleteCollectionInterceptor>,
    /// Intercept List operations
    pub list: Option<ListInterceptor>,
    /// Intercept Patch operations
    pub patch: Option<PatchInterceptor>,
    /// Intercept Watch operations
    pub watch: Option<WatchInterceptor>,
    /// Intercept Get Status subresource operations
    pub get_status: Option<GetStatusInterceptor>,
    /// Intercept Patch Status subresource operations
    pub patch_status: Option<PatchStatusInterceptor>,
    /// Intercept Replace Status subresource operations
    pub replace_status: Option<ReplaceStatusInterceptor>,
}

/// Context passed to Create interceptors
pub struct CreateContext<'a> {
    pub client: &'a FakeClient,
    /// The object being created
    pub object: &'a Value,
    /// Namespace for the object
    pub namespace: &'a str,
    /// Post parameters
    pub params: &'a PostParams,
}

/// Context passed to Get interceptors
pub struct GetContext<'a> {
    pub client: &'a FakeClient,
    /// Namespace of the object
    pub namespace: &'a str,
    /// Name of the object
    pub name: &'a str,
}

/// Context passed to Update interceptors
pub struct UpdateContext<'a> {
    pub client: &'a FakeClient,
    /// The updated object
    pub object: &'a Value,
    /// Namespace for the object
    pub namespace: &'a str,
    /// Whether this is a status subresource update
    pub is_status: bool,
    /// Post parameters
    pub params: &'a PostParams,
}

/// Context passed to Delete interceptors
pub struct DeleteContext<'a> {
    pub client: &'a FakeClient,
    /// Namespace of the object
    pub namespace: &'a str,
    /// Name of the object
    pub name: &'a str,
}

/// Context passed to List interceptors
pub struct ListContext<'a> {
    pub client: &'a FakeClient,
    pub namespace: Option<&'a str>,
    pub params: &'a ListParams,
}

/// Context passed to Patch interceptors
pub struct PatchContext<'a> {
    pub client: &'a FakeClient,
    /// The patch data to apply
    pub patch: &'a Value,
    /// Namespace of the object
    pub namespace: &'a str,
    /// Name of the object
    pub name: &'a str,
    /// Patch parameters
    pub params: &'a PatchParams,
}

pub type CreateInterceptor = Arc<dyn Fn(CreateContext) -> Result<Option<Value>> + Send + Sync>;

pub type GetInterceptor = Arc<dyn Fn(GetContext) -> Result<Option<Value>> + Send + Sync>;
pub type UpdateInterceptor = Arc<dyn Fn(UpdateContext) -> Result<Option<Value>> + Send + Sync>;
pub type DeleteInterceptor = Arc<dyn Fn(DeleteContext) -> Result<Option<Value>> + Send + Sync>;
pub type ListInterceptor = Arc<dyn Fn(ListContext) -> Result<Option<Vec<Value>>> + Send + Sync>;
pub type PatchInterceptor = Arc<dyn Fn(PatchContext) -> Result<Option<Value>> + Send + Sync>;

/// Context passed to Replace interceptors
pub struct ReplaceContext<'a> {
    pub client: &'a FakeClient,
    /// The replacement object
    pub object: &'a Value,
    /// Namespace for the object
    pub namespace: &'a str,
    /// Name of the object being replaced
    pub name: &'a str,
    /// Post parameters
    pub params: &'a PostParams,
}

pub type ReplaceInterceptor = Arc<dyn Fn(ReplaceContext) -> Result<Option<Value>> + Send + Sync>;

pub struct DeleteCollectionContext<'a> {
    pub client: &'a FakeClient,
    pub namespace: Option<&'a str>,
    pub params: &'a ListParams,
}

pub type DeleteCollectionInterceptor =
    Arc<dyn Fn(DeleteCollectionContext) -> Result<Option<Vec<Value>>> + Send + Sync>;

pub struct WatchContext<'a> {
    pub client: &'a FakeClient,
    pub namespace: Option<&'a str>,
    pub params: &'a ListParams,
}

pub type WatchInterceptor = Arc<dyn Fn(WatchContext) -> Result<Option<Vec<Value>>> + Send + Sync>;

pub struct GetStatusContext<'a> {
    pub client: &'a FakeClient,
    /// Namespace of the object
    pub namespace: &'a str,
    /// Name of the object
    pub name: &'a str,
}

pub type GetStatusInterceptor =
    Arc<dyn Fn(GetStatusContext) -> Result<Option<Value>> + Send + Sync>;

pub struct PatchStatusContext<'a> {
    pub client: &'a FakeClient,
    /// The patch data to apply
    pub patch: &'a Value,
    /// Namespace of the object
    pub namespace: &'a str,
    /// Name of the object
    pub name: &'a str,
    /// Patch parameters
    pub params: &'a PatchParams,
}

pub type PatchStatusInterceptor =
    Arc<dyn Fn(PatchStatusContext) -> Result<Option<Value>> + Send + Sync>;

pub struct ReplaceStatusContext<'a> {
    pub client: &'a FakeClient,
    /// The replacement object
    pub object: &'a Value,
    /// Namespace for the object
    pub namespace: &'a str,
    /// Name of the object
    pub name: &'a str,
    /// Post parameters
    pub params: &'a PostParams,
}

pub type ReplaceStatusInterceptor =
    Arc<dyn Fn(ReplaceStatusContext) -> Result<Option<Value>> + Send + Sync>;
