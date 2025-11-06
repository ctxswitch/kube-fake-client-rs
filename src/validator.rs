use crate::error::Result;
#[cfg(feature = "validation")]
use crate::error::Error;
use serde_json::Value;

/// Trait for schema validation implementations
pub trait SchemaValidator: Send + Sync {
    /// Validate a JSON value against the schema for a given GVK
    ///
    /// Takes group, version, and kind to uniquely identify the schema.
    /// For core resources, group is an empty string.
    fn validate(&self, group: &str, version: &str, kind: &str, value: &Value) -> Result<()>;
}


#[cfg(feature = "validation")]
mod runtime_openapi_validator {
    use super::*;
    use jsonschema::JSONSchema;
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::sync::RwLock;

    /// Runtime OpenAPI validator that loads schemas from OpenAPI spec files
    ///
    /// This validator allows developers to explicitly choose which resources to validate
    /// by loading schemas from OpenAPI spec files at runtime. Supports both Kubernetes
    /// built-in resources and CRDs with generated OpenAPI schemas.
    pub struct RuntimeOpenAPIValidator {
        /// Parsed OpenAPI definitions
        definitions: HashMap<String, Value>,
        /// Compiled schemas for registered GVKs (gvk_key -> schema)
        schemas: RwLock<HashMap<String, JSONSchema>>,
        /// Set of GVK keys that should be validated
        enabled_gvks: RwLock<Vec<String>>,
    }

    impl RuntimeOpenAPIValidator {
        /// Create a new validator from an OpenAPI spec file
        pub fn from_file<P: AsRef<Path>>(openapi_file: P) -> Result<Self> {
            let content = fs::read_to_string(openapi_file.as_ref()).map_err(|e| {
                Error::Internal(format!(
                    "Failed to read OpenAPI file {}: {}",
                    openapi_file.as_ref().display(),
                    e
                ))
            })?;

            let spec: Value = serde_json::from_str(&content).map_err(|e| {
                Error::Internal(format!("Failed to parse OpenAPI JSON: {}", e))
            })?;

            let definitions = spec
                .get("definitions")
                .and_then(|d| d.as_object())
                .ok_or_else(|| Error::Internal("OpenAPI spec missing 'definitions'".to_string()))?;

            let definitions_map: HashMap<String, Value> = definitions
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            Ok(Self {
                definitions: definitions_map,
                schemas: RwLock::new(HashMap::new()),
                enabled_gvks: RwLock::new(Vec::new()),
            })
        }

        pub fn enable_validation_for(&self, gvk: &str) -> Result<()> {
            let definition_name = self.gvk_to_definition_name(gvk)?;

            if !self.definitions.contains_key(&definition_name) {
                return Err(Error::Internal(format!(
                    "No OpenAPI definition found for GVK '{}' (looking for '{}')",
                    gvk, definition_name
                )));
            }

            self.enabled_gvks
                .write()
                .map_err(|e| Error::Internal(format!("Failed to acquire write lock: {}", e)))?
                .push(gvk.to_string());

            Ok(())
        }

        fn gvk_to_definition_name(&self, gvk: &str) -> Result<String> {
            let parts: Vec<&str> = gvk.trim_start_matches('/').split('/').collect();

            if parts.len() < 2 {
                return Err(Error::Internal(format!(
                    "Invalid GVK format '{}', expected 'group/version/Kind' or '/version/Kind'",
                    gvk
                )));
            }

            let (group, version, kind) = if parts.len() == 2 {
                ("", parts[0], parts[1])
            } else {
                (parts[0], parts[1], parts[2])
            };

            let def_name = if group.is_empty() {
                format!("io.k8s.api.core.{}.{}", version, kind)
            } else if group.contains('.') {
                let reversed_group: Vec<&str> = group.split('.').rev().collect();
                format!("{}.{}.{}", reversed_group.join("."), version, kind)
            } else {
                format!("io.k8s.api.{}.{}.{}", group, version, kind)
            };

            Ok(def_name)
        }

        fn get_or_compile_schema(&self, gvk_key: &str) -> Result<()> {
            {
                let cache = self
                    .schemas
                    .read()
                    .map_err(|e| Error::Internal(format!("Failed to acquire read lock: {}", e)))?;

                if cache.contains_key(gvk_key) {
                    return Ok(());
                }
            }

            let definition_name = self.gvk_to_definition_name(gvk_key)?;

            if !self.definitions.contains_key(&definition_name) {
                return Err(Error::Internal(format!(
                    "No definition found for GVK '{}' (definition: '{}')",
                    gvk_key, definition_name
                )));
            }

            let schema = serde_json::json!({
                "$schema": "http://json-schema.org/draft-04/schema#",
                "definitions": self.definitions,
                "$ref": format!("#/definitions/{}", definition_name)
            });

            let compiled = JSONSchema::compile(&schema).map_err(|e| {
                Error::Internal(format!(
                    "Failed to compile schema for '{}': {}",
                    gvk_key, e
                ))
            })?;

            self.schemas
                .write()
                .map_err(|e| Error::Internal(format!("Failed to acquire write lock: {}", e)))?
                .insert(gvk_key.to_string(), compiled);

            Ok(())
        }
    }

    impl SchemaValidator for RuntimeOpenAPIValidator {
        fn validate(&self, group: &str, version: &str, kind: &str, value: &Value) -> Result<()> {
            let gvk_key = if group.is_empty() {
                format!("/{}/{}", version, kind)
            } else {
                format!("{}/{}/{}", group, version, kind)
            };

            let enabled = self
                .enabled_gvks
                .read()
                .map_err(|e| Error::Internal(format!("Failed to acquire read lock: {}", e)))?
                .contains(&gvk_key);

            if !enabled {
                return Ok(());
            }

            self.get_or_compile_schema(&gvk_key)?;

            let schemas = self
                .schemas
                .read()
                .map_err(|e| Error::Internal(format!("Failed to acquire read lock: {}", e)))?;

            if let Some(schema) = schemas.get(&gvk_key) {
                let result = schema.validate(value);

                if let Err(validation_errors) = result {
                    let errors: Vec<String> = validation_errors
                        .map(|e| format!("{}: {}", e.instance_path, e))
                        .collect();

                    return Err(Error::ValidationFailed {
                        kind: kind.to_string(),
                        errors,
                    });
                }
            }

            Ok(())
        }
    }
}

#[cfg(feature = "validation")]
pub use runtime_openapi_validator::RuntimeOpenAPIValidator;

#[cfg(feature = "validation")]
impl SchemaValidator for std::sync::Arc<RuntimeOpenAPIValidator> {
    fn validate(&self, group: &str, version: &str, kind: &str, value: &Value) -> Result<()> {
        (**self).validate(group, version, kind, value)
    }
}

