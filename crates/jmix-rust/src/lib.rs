//! JMIX-RS: Rust library for JMIX (JSON Medical Interchange) format
//!
//! Secure medical data exchange with cryptographic features including:
//! - AES-256-GCM encryption with ECDH key exchange
//! - Ed25519 digital signatures and JWS support
//! - JSON Schema validation
//! - DICOM metadata extraction
//! - Envelope creation and processing

pub mod assertion;
pub mod builder;
pub mod config;
pub mod dicom;
pub mod encryption;
pub mod error;
pub mod jws;
pub mod types;
pub mod validation;
pub mod package_validation;

pub use package_validation::{validate_package, ValidationOptions, ValidationReport};

// Re-export commonly used types
pub use config::Config;
pub use error::{JmixError, JmixResult};
pub use types::{Envelope, Manifest, Metadata, Audit};
