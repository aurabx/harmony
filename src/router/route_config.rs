
#[derive(Debug, Clone)]
pub struct RouteConfig {
    pub path: String,
    pub methods: Vec<http::Method>, // E.g., GET, POST
    pub description: Option<String>, // Metadata for documentation or debugging
}
