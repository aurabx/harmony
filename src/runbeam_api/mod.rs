/// Runbeam Cloud API integration module
///
/// This module provides functionality for integrating Harmony with Runbeam Cloud,
/// including JWT validation, API client, and token storage.
///
/// ## Authorization Flow
///
/// 1. CLI calls Harmony Management API with user JWT token
/// 2. Harmony validates JWT locally (verifies signature and extracts claims)
/// 3. Harmony calls Runbeam Cloud API with user JWT to exchange for machine token
/// 4. Runbeam Cloud issues machine-scoped token (30-day expiry)
/// 5. Harmony stores machine token locally for future API calls

pub mod client;
pub mod jwt;
pub mod token_storage;
pub mod types;

pub use client::RunbeamClient;
pub use jwt::{validate_jwt_token, JwtClaims};
pub use token_storage::{clear_token, load_token, save_token, MachineToken};
pub use types::{ApiError, RunbeamError};
