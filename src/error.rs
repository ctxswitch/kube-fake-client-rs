use kube::error::ErrorResponse;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
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
}

impl Error {
    /// Convert internal error to kube::Error for API compatibility
    /// This ensures fake client returns the same error types as real kube client
    /// with exact message formats matching Kubernetes API
    pub fn into_kube_err(self) -> kube::Error {
        let error_response = match &self {
            // Format: 'pods "my-pod" not found'
            Error::NotFound { kind, name, .. } => ErrorResponse {
                status: "Failure".to_string(),
                message: format!("{} \"{}\" not found", kind, name),
                reason: "NotFound".to_string(),
                code: 404,
            },
            // Format: 'pods "my-pod" already exists'
            Error::AlreadyExists { kind, name, .. } => ErrorResponse {
                status: "Failure".to_string(),
                message: format!("{} \"{}\" already exists", kind, name),
                reason: "AlreadyExists".to_string(),
                code: 409,
            },
            Error::Conflict(msg) => ErrorResponse {
                status: "Failure".to_string(),
                message: msg.clone(),
                reason: "Conflict".to_string(),
                code: 409,
            },
            Error::InvalidRequest(msg) => ErrorResponse {
                status: "Failure".to_string(),
                message: msg.clone(),
                reason: "Invalid".to_string(),
                code: 422,
            },
            Error::IndexNotFound { kind, field } => ErrorResponse {
                status: "Failure".to_string(),
                message: format!("field selector {} not supported for {}", field, kind),
                reason: "BadRequest".to_string(),
                code: 400,
            },
            Error::SerializationError(e) => ErrorResponse {
                status: "Failure".to_string(),
                message: format!("Serialization error: {}", e),
                reason: "BadRequest".to_string(),
                code: 400,
            },
            Error::PatchError(e) => ErrorResponse {
                status: "Failure".to_string(),
                message: format!("Patch error: {}", e),
                reason: "Invalid".to_string(),
                code: 422,
            },
            Error::MetadataError(msg) => ErrorResponse {
                status: "Failure".to_string(),
                message: msg.clone(),
                reason: "Invalid".to_string(),
                code: 400,
            },
            Error::Internal(msg) => ErrorResponse {
                status: "Failure".to_string(),
                message: msg.clone(),
                reason: "InternalError".to_string(),
                code: 500,
            },
        };

        kube::Error::Api(error_response)
    }
}
