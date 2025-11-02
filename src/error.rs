use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Resource not found: {kind} {name} in namespace {namespace}")]
    NotFound {
        kind: String,
        name: String,
        namespace: String,
    },

    #[error("Resource already exists: {kind} {name} in namespace {namespace}")]
    AlreadyExists {
        kind: String,
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
