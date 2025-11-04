//! Interceptors for customizing client behavior during testing

use crate::client::FakeClient;
use crate::Result;
use kube::api::{ListParams, PatchParams, PostParams};
use serde_json::Value;
use std::sync::Arc;

/// Interceptor functions for client operations
///
/// Return `Ok(Some(value))` to override, `Ok(None)` to continue, or `Err(e)` to inject an error.
///
/// # Example
/// ```
/// use kube_fake_client::interceptor;
///
/// let funcs = interceptor::Funcs::new()
///     .create(|ctx| {
///         // Custom create logic
///         Ok(None)
///     })
///     .get(|ctx| {
///         // Custom get logic
///         Ok(None)
///     });
/// ```
#[derive(Default)]
pub struct Funcs {
    /// Intercept Create operations
    pub(crate) create: Option<CreateInterceptor>,
    /// Intercept Get operations
    pub(crate) get: Option<GetInterceptor>,
    /// Intercept Update operations (PATCH-based updates)
    pub(crate) update: Option<UpdateInterceptor>,
    /// Intercept Replace operations (PUT - full replacement)
    pub(crate) replace: Option<ReplaceInterceptor>,
    /// Intercept Delete operations
    pub(crate) delete: Option<DeleteInterceptor>,
    /// Intercept Delete Collection operations
    pub(crate) delete_collection: Option<DeleteCollectionInterceptor>,
    /// Intercept List operations
    pub(crate) list: Option<ListInterceptor>,
    /// Intercept Patch operations
    pub(crate) patch: Option<PatchInterceptor>,
    /// Intercept Watch operations
    pub(crate) watch: Option<WatchInterceptor>,
    /// Intercept Get Status subresource operations
    pub(crate) get_status: Option<GetStatusInterceptor>,
    /// Intercept Patch Status subresource operations
    pub(crate) patch_status: Option<PatchStatusInterceptor>,
    /// Intercept Replace Status subresource operations
    pub(crate) replace_status: Option<ReplaceStatusInterceptor>,
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

impl Funcs {
    /// Create a new empty set of interceptors
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a Create interceptor
    pub fn create<F>(mut self, f: F) -> Self
    where
        F: Fn(CreateContext) -> Result<Option<Value>> + Send + Sync + 'static,
    {
        self.create = Some(Arc::new(f));
        self
    }

    /// Add a Get interceptor
    pub fn get<F>(mut self, f: F) -> Self
    where
        F: Fn(GetContext) -> Result<Option<Value>> + Send + Sync + 'static,
    {
        self.get = Some(Arc::new(f));
        self
    }

    /// Add an Update interceptor
    pub fn update<F>(mut self, f: F) -> Self
    where
        F: Fn(UpdateContext) -> Result<Option<Value>> + Send + Sync + 'static,
    {
        self.update = Some(Arc::new(f));
        self
    }

    /// Add a Replace interceptor
    pub fn replace<F>(mut self, f: F) -> Self
    where
        F: Fn(ReplaceContext) -> Result<Option<Value>> + Send + Sync + 'static,
    {
        self.replace = Some(Arc::new(f));
        self
    }

    /// Add a Delete interceptor
    pub fn delete<F>(mut self, f: F) -> Self
    where
        F: Fn(DeleteContext) -> Result<Option<Value>> + Send + Sync + 'static,
    {
        self.delete = Some(Arc::new(f));
        self
    }

    /// Add a Delete Collection interceptor
    pub fn delete_collection<F>(mut self, f: F) -> Self
    where
        F: Fn(DeleteCollectionContext) -> Result<Option<Vec<Value>>> + Send + Sync + 'static,
    {
        self.delete_collection = Some(Arc::new(f));
        self
    }

    /// Add a List interceptor
    pub fn list<F>(mut self, f: F) -> Self
    where
        F: Fn(ListContext) -> Result<Option<Vec<Value>>> + Send + Sync + 'static,
    {
        self.list = Some(Arc::new(f));
        self
    }

    /// Add a Patch interceptor
    pub fn patch<F>(mut self, f: F) -> Self
    where
        F: Fn(PatchContext) -> Result<Option<Value>> + Send + Sync + 'static,
    {
        self.patch = Some(Arc::new(f));
        self
    }

    /// Add a Watch interceptor
    pub fn watch<F>(mut self, f: F) -> Self
    where
        F: Fn(WatchContext) -> Result<Option<Vec<Value>>> + Send + Sync + 'static,
    {
        self.watch = Some(Arc::new(f));
        self
    }

    /// Add a Get Status interceptor
    pub fn get_status<F>(mut self, f: F) -> Self
    where
        F: Fn(GetStatusContext) -> Result<Option<Value>> + Send + Sync + 'static,
    {
        self.get_status = Some(Arc::new(f));
        self
    }

    /// Add a Patch Status interceptor
    pub fn patch_status<F>(mut self, f: F) -> Self
    where
        F: Fn(PatchStatusContext) -> Result<Option<Value>> + Send + Sync + 'static,
    {
        self.patch_status = Some(Arc::new(f));
        self
    }

    /// Add a Replace Status interceptor
    pub fn replace_status<F>(mut self, f: F) -> Self
    where
        F: Fn(ReplaceStatusContext) -> Result<Option<Value>> + Send + Sync + 'static,
    {
        self.replace_status = Some(Arc::new(f));
        self
    }
}
