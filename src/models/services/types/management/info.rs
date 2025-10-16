use axum::response::Json;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
pub struct InfoResponse {
    version: String,
    uptime: u64,
    os: String,
    arch: String,
}

pub async fn handle_info() -> Json<InfoResponse> {
    let start = SystemTime::now();
    let uptime = start
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Json(InfoResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    })
}
