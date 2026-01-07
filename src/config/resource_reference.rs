//! Resource reference parsing for mesh ingress/egress validation.
//!
//! Supports the provider-based reference format:
//! - `<name>` - Bare name (local.name.<name>)
//! - `local.name.<name>` - Explicit local lookup
//! - `<provider>.id.<id>` - Provider-wide ID lookup
//! - `<provider>.<team>.id.<id>` - Team-scoped ID lookup
//! - `<provider>.<team>.<type>.name.<name>` - Full path by name
//! - `<provider>.<team>.<type>.id.<id>` - Full path by ID

/// Parsed resource reference
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReference {
    /// Provider name (e.g., "local", "runbeam")
    pub provider: String,
    /// Optional team identifier
    pub team: Option<String>,
    /// Optional resource type (ingress, egress, etc.)
    pub resource_type: Option<String>,
    /// How to look up the resource
    pub lookup: LookupBy,
}

/// How to look up the resource
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupBy {
    /// Lookup by ULID
    Id(String),
    /// Lookup by name
    Name(String),
}

impl ParsedReference {
    /// Parse a resource reference string.
    ///
    /// # Examples
    /// ```
    /// use harmony::config::resource_reference::ParsedReference;
    ///
    /// // Bare name
    /// let r = ParsedReference::parse("my_ingress").unwrap();
    /// assert_eq!(r.provider, "local");
    ///
    /// // Full path
    /// let r = ParsedReference::parse("runbeam.acme.ingress.name.patient_api").unwrap();
    /// assert_eq!(r.provider, "runbeam");
    /// assert_eq!(r.team, Some("acme".to_string()));
    /// ```
    pub fn parse(input: &str) -> Result<Self, String> {
        let parts: Vec<&str> = input.split('.').collect();

        match parts.len() {
            // Bare name: "my_ingress" -> local.name.my_ingress
            1 => Ok(ParsedReference {
                provider: "local".to_string(),
                team: None,
                resource_type: None,
                lookup: LookupBy::Name(parts[0].to_string()),
            }),

            // "local.name.{name}" or "{provider}.id.{id}"
            3 => {
                let provider = parts[0];
                match parts[1] {
                    "name" => Ok(ParsedReference {
                        provider: provider.to_string(),
                        team: None,
                        resource_type: None,
                        lookup: LookupBy::Name(parts[2].to_string()),
                    }),
                    "id" => Ok(ParsedReference {
                        provider: provider.to_string(),
                        team: None,
                        resource_type: None,
                        lookup: LookupBy::Id(parts[2].to_string()),
                    }),
                    _ => Err(format!(
                        "Invalid reference format: expected 'name' or 'id', got '{}'",
                        parts[1]
                    )),
                }
            }

            // "{provider}.{team}.id.{id}"
            4 => {
                let provider = parts[0];
                let team = parts[1];
                match parts[2] {
                    "id" => Ok(ParsedReference {
                        provider: provider.to_string(),
                        team: Some(team.to_string()),
                        resource_type: None,
                        lookup: LookupBy::Id(parts[3].to_string()),
                    }),
                    _ => Err(format!(
                        "Invalid reference format: expected 'id' at position 2, got '{}'",
                        parts[2]
                    )),
                }
            }

            // "{provider}.{team}.{type}.name.{name}" or "{provider}.{team}.{type}.id.{id}"
            5 => {
                let provider = parts[0];
                let team = parts[1];
                let resource_type = parts[2];

                // Validate resource type
                let valid_types = ["ingress", "egress", "pipeline", "endpoint", "backend", "mesh"];
                if !valid_types.contains(&resource_type) {
                    return Err(format!("Invalid resource type: {}", resource_type));
                }

                match parts[3] {
                    "name" => Ok(ParsedReference {
                        provider: provider.to_string(),
                        team: Some(team.to_string()),
                        resource_type: Some(resource_type.to_string()),
                        lookup: LookupBy::Name(parts[4].to_string()),
                    }),
                    "id" => Ok(ParsedReference {
                        provider: provider.to_string(),
                        team: Some(team.to_string()),
                        resource_type: Some(resource_type.to_string()),
                        lookup: LookupBy::Id(parts[4].to_string()),
                    }),
                    _ => Err(format!(
                        "Invalid reference format: expected 'name' or 'id', got '{}'",
                        parts[3]
                    )),
                }
            }

            // Invalid format
            _ => Err(format!(
                "Invalid reference format: unexpected number of parts ({})",
                parts.len()
            )),
        }
    }

    /// Returns true if this is a local reference (can be resolved from local config)
    pub fn is_local(&self) -> bool {
        self.provider == "local"
    }

    /// Returns true if this is a remote reference (requires provider API call)
    pub fn is_remote(&self) -> bool {
        self.provider != "local"
    }

    /// Get the lookup name for local resolution
    pub fn local_name(&self) -> Option<&str> {
        if self.is_local() {
            if let LookupBy::Name(ref name) = self.lookup {
                return Some(name);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bare_name() {
        let r = ParsedReference::parse("my_ingress").unwrap();
        assert_eq!(r.provider, "local");
        assert!(r.team.is_none());
        assert!(r.resource_type.is_none());
        assert_eq!(r.lookup, LookupBy::Name("my_ingress".to_string()));
        assert!(r.is_local());
        assert_eq!(r.local_name(), Some("my_ingress"));
    }

    #[test]
    fn test_parse_local_name() {
        let r = ParsedReference::parse("local.name.fhir_api").unwrap();
        assert_eq!(r.provider, "local");
        assert!(r.team.is_none());
        assert!(r.resource_type.is_none());
        assert_eq!(r.lookup, LookupBy::Name("fhir_api".to_string()));
        assert!(r.is_local());
    }

    #[test]
    fn test_parse_provider_id() {
        let r = ParsedReference::parse("runbeam.id.01JGXYZ123ABC").unwrap();
        assert_eq!(r.provider, "runbeam");
        assert!(r.team.is_none());
        assert!(r.resource_type.is_none());
        assert_eq!(r.lookup, LookupBy::Id("01JGXYZ123ABC".to_string()));
        assert!(r.is_remote());
    }

    #[test]
    fn test_parse_provider_team_id() {
        let r = ParsedReference::parse("runbeam.acme.id.01JGXYZ123ABC").unwrap();
        assert_eq!(r.provider, "runbeam");
        assert_eq!(r.team, Some("acme".to_string()));
        assert!(r.resource_type.is_none());
        assert_eq!(r.lookup, LookupBy::Id("01JGXYZ123ABC".to_string()));
    }

    #[test]
    fn test_parse_full_path_by_name() {
        let r = ParsedReference::parse("runbeam.acme.ingress.name.patient_api").unwrap();
        assert_eq!(r.provider, "runbeam");
        assert_eq!(r.team, Some("acme".to_string()));
        assert_eq!(r.resource_type, Some("ingress".to_string()));
        assert_eq!(r.lookup, LookupBy::Name("patient_api".to_string()));
    }

    #[test]
    fn test_parse_full_path_by_id() {
        let r = ParsedReference::parse("runbeam.acme.egress.id.01JGXYZ123ABC").unwrap();
        assert_eq!(r.provider, "runbeam");
        assert_eq!(r.team, Some("acme".to_string()));
        assert_eq!(r.resource_type, Some("egress".to_string()));
        assert_eq!(r.lookup, LookupBy::Id("01JGXYZ123ABC".to_string()));
    }

    #[test]
    fn test_parse_invalid_reference() {
        // Two parts is invalid
        assert!(ParsedReference::parse("invalid.format").is_err());

        // Invalid lookup type
        assert!(ParsedReference::parse("local.invalid.test").is_err());

        // Invalid resource type
        assert!(ParsedReference::parse("runbeam.acme.invalid.name.test").is_err());
    }

    #[test]
    fn test_all_valid_resource_types() {
        for resource_type in ["ingress", "egress", "pipeline", "endpoint", "backend", "mesh"] {
            let ref_str = format!("runbeam.acme.{}.name.test", resource_type);
            let r = ParsedReference::parse(&ref_str).unwrap();
            assert_eq!(r.resource_type, Some(resource_type.to_string()));
        }
    }
}
