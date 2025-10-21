/// # Routing Middleware Example
///
/// This example demonstrates how middleware can use the RequestEnvelope helper methods
/// to modify target_details, enabling dynamic routing, path rewriting, and header injection.
///
/// ## Use Cases
///
/// 1. **Dynamic Backend Selection**: Route requests to different backends based on headers,
///    query params, or request content.
///
/// 2. **Path Rewriting**: Transform the request path before it reaches the backend.
///
/// 3. **Header Injection**: Add authentication or routing headers to backend requests.
///
/// 4. **Protocol Translation**: Set backend-specific metadata (e.g., DIMSE operations).
///
/// ## When to Use target_details vs request_details
///
/// - **request_details**: Contains the original incoming request information. Middleware
///   should read from this to understand the client's request.
///
/// - **target_details**: Contains the information that will be sent to the backend. Middleware
///   should modify this to control what the backend receives.
///
/// The key difference is that `request_details` preserves the original request state,
/// while `target_details` represents what will actually be sent to the backend.

// Note: This is a documentation file demonstrating patterns for middleware development.
// These examples show the target_details API but are not meant to be compiled as-is.
// In actual middleware implementation within the harmony crate, you would have access
// to the proper types and imports.

fn main() {
    println!("# Routing Middleware Patterns");
    println!();
    println!("This file documents patterns for using RequestEnvelope's target_details methods.");
    println!("See the code below for examples and explanations.");
    println!();
    println!("## Available Methods:");
    println!("- envelope.set_target_base_url(url) - Override backend URL");
    println!("- envelope.set_target_uri(path) - Rewrite request path");
    println!("- envelope.set_target_header(key, value) - Add/modify headers");
    println!("- envelope.set_target_query_param(key, values) - Add/modify query params");
    println!("- envelope.set_target_metadata(key, value) - Set backend-specific metadata");
    println!("- envelope.set_target_method(method) - Change HTTP method");
}

#[cfg(doc)]
mod examples {
    //! Example middleware implementations demonstrating target_details usage.
    //!
    //! These are documentation examples showing the API patterns.
    
    use std::collections::HashMap;
    
    /// Example: Route to different backends based on X-Tenant-ID header
    ///
    /// ```rust,no_run
    /// # use harmony::models::envelope::envelope::RequestEnvelope;
    /// # use harmony::models::middleware::middleware::Middleware;
    /// # async fn example(mut envelope: RequestEnvelope<serde_json::Value>) {
    /// // Read the tenant ID from the original request headers
    /// let tenant_id = envelope
    ///     .request_details
    ///     .headers
    ///     .get("x-tenant-id");
    /// 
    /// // Route to appropriate backend
    /// match tenant_id.map(|s| s.as_str()) {
    ///     Some("tenant-a") => {
    ///         envelope.set_target_base_url("https://api-a.example.com");
    ///     }
    ///     Some("tenant-b") => {
    ///         envelope.set_target_base_url("https://api-b.example.com");
    ///     }
    ///     _ => {
    ///         envelope.set_target_base_url("https://api-default.example.com");
    ///     }
    /// }
    /// # }
    /// ```
    pub struct TenantRoutingExample;
    
    /// Example: Rewrite API paths from v1 to v2
    ///
    /// ```rust,no_run
    /// # use harmony::models::envelope::envelope::RequestEnvelope;
    /// # async fn example(mut envelope: RequestEnvelope<serde_json::Value>) {
    /// // Read the original URI
    /// let original_uri = &envelope.request_details.uri;
    /// 
    /// // Rewrite /v1/... to /v2/...
    /// if original_uri.starts_with("/v1/") {
    ///     let new_uri = original_uri.replace("/v1/", "/v2/");
    ///     envelope.set_target_uri(&new_uri);
    /// }
    /// # }
    /// ```
    pub struct ApiVersionRewriteExample;
    
    /// Example: Add authentication headers to backend requests
    ///
    /// ```rust,no_run
    /// # use harmony::models::envelope::envelope::RequestEnvelope;
    /// # async fn example(mut envelope: RequestEnvelope<serde_json::Value>, auth_token: &str) {
    /// // Add an Authorization header that will be sent to the backend
    /// envelope.set_target_header("Authorization", format!("Bearer {}", auth_token));
    /// 
    /// // Could also add custom headers for routing or tracing
    /// envelope.set_target_header("X-Proxy-Version", "1.0");
    /// # }
    /// ```
    pub struct AuthInjectionExample;
    
    /// Example: Set DICOM operation based on FHIR resource type
    ///
    /// ```rust,no_run
    /// # use harmony::models::envelope::envelope::RequestEnvelope;
    /// # async fn example(mut envelope: RequestEnvelope<serde_json::Value>) {
    /// // Check if this is a FHIR ImagingStudy query
    /// let is_imaging_study = envelope.request_details.uri.contains("/ImagingStudy");
    /// 
    /// if is_imaging_study {
    ///     // Set the DICOM operation metadata for the backend
    ///     // The DICOM backend checks target_details.metadata["dimse_op"] first
    ///     envelope.set_target_metadata("dimse_op", "find");
    /// }
    /// # }
    /// ```
    pub struct FhirToDicomExample;
    
    /// Example: Composite middleware combining multiple routing strategies
    ///
    /// ```rust,no_run
    /// # use harmony::models::envelope::envelope::RequestEnvelope;
    /// # async fn example(mut envelope: RequestEnvelope<serde_json::Value>) {
    /// // Check multiple conditions to determine routing
    /// let is_high_priority = envelope
    ///     .request_details
    ///     .headers
    ///     .get("x-priority")
    ///     .map(|v| v == "high")
    ///     .unwrap_or(false);
    /// 
    /// let is_urgent_path = envelope.request_details.uri.contains("/urgent/");
    /// 
    /// // Route to high-priority backend if needed
    /// if is_high_priority || is_urgent_path {
    ///     envelope.set_target_base_url("https://high-priority.example.com");
    ///     envelope.set_target_header("X-Priority-Request", "true");
    ///     envelope.set_target_query_param("priority", vec!["high".to_string()]);
    /// } else {
    ///     envelope.set_target_base_url("https://normal.example.com");
    /// }
    /// # }
    /// ```
    pub struct SmartRoutingExample;
}
