//! Extension points for commercial and custom Harmony editions.
//!
//! This module provides the foundation for building custom Harmony editions
//! by exposing builder patterns, lifecycle hooks, and factory traits.
//!
//! # Architecture
//!
//! The extension system uses a unified registry where all middleware and service
//! types (both built-in and custom) are registered as factories. This makes
//! built-ins and custom extensions equal citizens.
//!
//! Commercial editions depend on the open-source core as a library:
//!
//! ```text
//! harmony (public)
//!     ^
//!     |
//! harmony-commercial (private)
//!     ^
//!     |
//! harmony-aurabox (private)
//! ```
//!
//! # Usage Example
//!
//! ```rust,ignore
//! use harmony::{Config, HarmonyBuilder};
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = Config::load("config.toml").unwrap();
//!     
//!     HarmonyBuilder::new(config)
//!         // Register custom auth providers
//!         .with_auth_provider(Box::new(SamlProvider::new()))
//!         // Register custom middleware factories
//!         .with_middleware_factory(Box::new(RateLimitFactory::new()))
//!         // Register custom service factories
//!         .with_service_factory(Box::new(CloudStorageFactory::new()))
//!         // Register config extensions for validation
//!         .with_config_extension(Box::new(LicenseExtension::new()))
//!         // Register plugins for lifecycle hooks
//!         .with_plugin(Box::new(AuditPlugin::new()))
//!         .run()
//!         .await;
//! }
//! ```

mod auth;
pub mod builtin;
mod builder;
mod config_extension;
mod middleware;
mod plugin;
pub mod registry;
mod service;

pub use auth::AuthProvider;
pub use builder::{get_extension_registry, ExtensionRegistry, HarmonyBuilder};
pub use config_extension::ConfigExtension;
pub use middleware::MiddlewareFactory;
pub use plugin::HarmonyPlugin;
pub use registry::{get_registry, initialize_builtin_factories, UnifiedRegistry};
pub use service::ServiceFactory;
