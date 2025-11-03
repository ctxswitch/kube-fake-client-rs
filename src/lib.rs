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
//!
//! ## Cluster-Scoped Resources
//!
//! ```rust
//! use kube_fake_client::ClientBuilder;
//! use k8s_openapi::api::core::v1::Node;
//! use kube::api::{Api, PostParams};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let client = ClientBuilder::new().build().await?;
//! let nodes: Api<Node> = Api::all(client);
//!
//! let mut node = Node::default();
//! node.metadata.name = Some("worker-1".to_string());
//!
//! nodes.create(&PostParams::default(), &node).await?;
//! # Ok(())
//! # }
//! ```

mod builder;
mod client;
mod client_utils;
mod error;
mod field_selectors;
pub mod interceptor;
pub mod label_selector;
mod mock_service;
mod tracker;
mod utils;

#[cfg(test)]
mod builder_test;
#[cfg(test)]
mod client_test;
#[cfg(test)]
mod client_utils_test;
#[cfg(test)]
mod mock_service_test;
#[cfg(test)]
mod tracker_test;
#[cfg(test)]
mod utils_test;

pub use builder::ClientBuilder;
pub use error::{Error, Result};
pub use kube::Client;
