//! Protocol-agnostic content detection and classification.
//!
//! This module provides content detection logic that works across all protocol
//! adapters. It determines whether content is binary or text-based using:
//!
//! 1. MIME type hints from protocol metadata
//! 2. Content-Encoding detection (compressed content)
//! 3. Byte-level inspection (magic bytes, BOM markers, byte distribution)
//!
//! The key question this module answers is: "Should I parse/transform this
//! content, or pass it through as opaque bytes?"
//!
//! # Usage
//!
//! ```rust,ignore
//! use harmony::core::content::{ContentAnalysis, ContentDisposition};
//!
//! let analysis = ContentAnalysis {
//!     mime_type: Some("application/json"),
//!     encoding: None,
//!     body_sample: None,
//! };
//!
//! match analysis.disposition() {
//!     ContentDisposition::Text => { /* safe to parse/transform */ }
//!     ContentDisposition::Binary => { /* pass through untouched */ }
//! }
//! ```

mod analysis;
mod detection;
pub mod mime;

pub use analysis::{ContentAnalysis, ContentDisposition};
pub use detection::{sniff, sniff_conservative};
