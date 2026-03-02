use kube::core::response::StatusSummary;
use kube::core::Status;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("Resource not found: {kind} {name} in namespace {namespace}")]
    NotFound {
        kind: String, // resource name (lowercase plural, e.g., "pods")
        name: String,
        namespace: String,
    },

    #[error("Resource already exists: {kind} {name} in namespace {namespace}")]
    AlreadyExists {
        kind: String, // resource name (lowercase plural, e.g., "pods")
        name: String,
        namespace: String,
    },

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("JSON patch error: {0}")]
    PatchError(#[from] json_patch::PatchError),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Failed to access object metadata: {0}")]
    MetadataError(String),

    #[error("Index {field} not registered for {kind}")]
    IndexNotFound { kind: String, field: String },

    #[error("Resource type not registered: {group}/{version}/{resource}")]
    ResourceNotRegistered {
        group: String,
        version: String,
        resource: String,
    },

    #[error("Verb {verb} not supported for resource {kind}")]
    VerbNotSupported { verb: String, kind: String },

    #[error("Schema validation failed for {kind}: {errors:?}")]
    ValidationFailed { kind: String, errors: Vec<String> },

    #[error("Immutable field cannot be changed: {field}")]
    ImmutableField { field: String },
}

impl Error {
    /// Convert internal error to kube::Error for API compatibility
    /// This ensures fake client returns the same error types as real kube client
    /// with exact message formats matching Kubernetes API
    pub fn into_kube_err(self) -> kube::Error {
        let error_response = match &self {
            // Format: 'pods "my-pod" not found'
            Error::NotFound { kind, name, .. } => Status {
                status: Some(StatusSummary::Failure),
                message: format!("{kind} \"{name}\" not found"),
                reason: "NotFound".to_string(),
                code: 404,
                metadata: None,
                details: None,
            },
            // Format: 'pods "my-pod" already exists'
            Error::AlreadyExists { kind, name, .. } => Status {
                status: Some(StatusSummary::Failure),
                message: format!("{kind} \"{name}\" already exists"),
                reason: "AlreadyExists".to_string(),
                code: 409,
                metadata: None,
                details: None,
            },
            Error::Conflict(msg) => Status {
                status: Some(StatusSummary::Failure),
                message: msg.clone(),
                reason: "Conflict".to_string(),
                code: 409,
                metadata: None,
                details: None,
            },
            Error::InvalidRequest(msg) | Error::MetadataError(msg) => Status {
                status: Some(StatusSummary::Failure),
                message: msg.clone(),
                reason: "Invalid".to_string(),
                code: 422,
                metadata: None,
                details: None,
            },
            Error::Internal(msg) => Status {
                status: Some(StatusSummary::Failure),
                message: msg.clone(),
                reason: "InternalError".to_string(),
                code: 500,
                metadata: None,
                details: None,
            },
            Error::IndexNotFound { kind, field } => Status {
                status: Some(StatusSummary::Failure),
                message: format!("field selector {field} not supported for {kind}"),
                reason: "BadRequest".to_string(),
                code: 400,
                metadata: None,
                details: None,
            },
            Error::SerializationError(e) => Status {
                status: Some(StatusSummary::Failure),
                message: format!("Serialization error: {e}"),
                reason: "BadRequest".to_string(),
                code: 400,
                metadata: None,
                details: None,
            },
            Error::PatchError(e) => Status {
                status: Some(StatusSummary::Failure),
                message: format!("Patch error: {e}"),
                reason: "Invalid".to_string(),
                code: 422,
                metadata: None,
                details: None,
            },
            Error::ResourceNotRegistered {
                group,
                version: _,
                resource,
            } => {
                let group_prefix = if group.is_empty() {
                    String::new()
                } else {
                    format!("{group}/")
                };
                Status {
                    status: Some(StatusSummary::Failure),
                    message: format!("the server could not find the requested resource ({group_prefix}{resource})"),
                    reason: "NotFound".to_string(),
                    code: 404,
                    metadata: None,
                    details: None,
                }
            }
            Error::VerbNotSupported { verb, kind } => Status {
                status: Some(StatusSummary::Failure),
                message: format!(
                    "{kind} \"{kind}\" is forbidden: verb \"{verb}\" is not supported"
                ),
                reason: "MethodNotAllowed".to_string(),
                code: 405,
                metadata: None,
                details: None,
            },
            Error::ValidationFailed { kind, errors } => {
                let errors_str = errors.join(", ");
                Status {
                    status: Some(StatusSummary::Failure),
                    message: format!("{kind} failed schema validation: {errors_str}"),
                    reason: "Invalid".to_string(),
                    code: 422,
                    metadata: None,
                    details: None,
                }
            }
            Error::ImmutableField { field } => Status {
                status: Some(StatusSummary::Failure),
                message: format!("field is immutable: {field}"),
                reason: "Invalid".to_string(),
                code: 422,
                metadata: None,
                details: None,
            },
        };

        kube::Error::Api(Box::new(error_response))
    }
}
