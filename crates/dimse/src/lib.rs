//! DIMSE (DICOM Message Service Element) implementation
//!
//! This crate provides both Service Class Provider (SCP) and Service Class User (SCU)
//! implementations for DICOM networking using the DIMSE protocol.
//!
//! # Features
//! - Inbound DIMSE services (SCP): C-ECHO, C-FIND, C-MOVE
//! - Outbound DIMSE services (SCU): C-ECHO, C-FIND, C-MOVE  
//! - TLS support (optional, feature = "tls")
//! - Binary stream handling with minimal file I/O
//! - Integration with harmony proxy via internal router

pub mod config;
pub mod error;
pub mod router;
pub mod scp;
pub mod scu;
pub mod types;

#[cfg(feature = "tls")]
pub mod tls;

// Re-export commonly used types
pub use config::{DimseConfig, RemoteNode};
pub use error::{DimseError, Result};
pub use router::{DimseRequest, DimseResponse, InMemoryRouter, Router};
pub use scp::DimseScp;
pub use scu::DimseScu;
pub use types::{DatasetStream, DimseCommand};

/// DIMSE protocol version
pub const DIMSE_VERSION: &str = "0.1.0";

/// Default DICOM port (non-TLS)
pub const DEFAULT_DIMSE_PORT: u16 = 11112;

/// Default TLS DICOM port
pub const DEFAULT_DIMSE_TLS_PORT: u16 = 2762;
