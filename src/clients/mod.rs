//! Client modules for outbound connections.
//!
//! This module provides HTTP clients for making outbound requests to upstream servers.

pub mod http3;

pub use http3::Http3Client;
