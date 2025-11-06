//! In-memory Kubernetes client for testing controllers and operators.
//!
//! Based on controller-runtime's fake client from the Go ecosystem.
//!
//! # Examples
//!
//! ## Namespaced Resources
//!
//! ```rust
//! use kube_fake_client::ClientBuilder;
//! use k8s_openapi::api::core::v1::Pod;
//! use kube::api::{Api, PostParams};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let client = ClientBuilder::new().build().await?;
//! let pods: Api<Pod> = Api::namespaced(client, "default");
//!
//! let mut pod = Pod::default();
//! pod.metadata.name = Some("test-pod".to_string());
//!
//! pods.create(&PostParams::default(), &pod).await?;
//! # Ok(())
//! # }
//! ```

mod builder;
mod client;
mod client_utils;
pub mod discovery;
mod error;
mod field_selectors;
pub mod gen;
pub mod interceptor;
pub mod label_selector;
mod mock_service;
pub mod registry;
mod tracker;
mod utils;
pub mod validator;

#[cfg(test)]
mod builder_test;
#[cfg(test)]
mod client_test;
#[cfg(test)]
mod mock_service_test;
#[cfg(test)]
mod tracker_test;
#[cfg(test)]
mod utils_test;

pub use builder::ClientBuilder;
pub use error::{Error, Result};
pub use kube::Client;
