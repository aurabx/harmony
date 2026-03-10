//! Core protocol-agnostic modules for Harmony proxy.
//!
//! This module contains shared logic that is used across multiple protocol
//! adapters (HTTP, HTTP/3, DIMSE, etc.) without coupling to any specific protocol.

pub mod content;
