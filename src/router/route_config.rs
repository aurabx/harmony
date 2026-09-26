#[derive(Debug, Clone)]
pub struct RouteConfig {
    pub path: String,
    pub methods: Vec<http::Method>,  // E.g., GET, POST
    pub description: Option<String>, // Metadata for documentation or debugging
}

/// Methods a general-purpose HTTP endpoint hands to its pipeline.
///
/// Generic endpoints (`http`, `http_streaming`, `echo`) do not restrict
/// methods themselves. Policies and backends decide what is allowed.
///
/// TRACE and CONNECT are left out. TRACE reflects the request back to the
/// client, including credential headers, and has no use through a proxy.
/// CONNECT asks for a raw TCP tunnel, which the pipeline cannot represent
/// and the backends cannot forward.
pub fn standard_methods() -> Vec<http::Method> {
    vec![
        http::Method::GET,
        http::Method::HEAD,
        http::Method::POST,
        http::Method::PUT,
        http::Method::PATCH,
        http::Method::DELETE,
        http::Method::OPTIONS,
    ]
}
