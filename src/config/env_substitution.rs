use regex::Regex;
use std::env;
use std::collections::HashMap;
use once_cell::sync::Lazy;

static ENV_VAR_REGEX: Lazy<Regex> = Lazy::new(|| {
    // Matches $VAR_NAME where VAR_NAME starts with letter or underscore
    // followed by alphanumeric or underscore characters
    Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)").expect("Failed to compile environment variable regex")
});

/// Result type for environment variable substitution operations
pub type SubstitutionResult<T> = Result<T, SubstitutionError>;

/// Errors that can occur during environment variable substitution
#[derive(Debug, Clone)]
pub struct SubstitutionError {
    pub missing_vars: Vec<String>,
    pub message: String,
}

impl std::fmt::Display for SubstitutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}. Missing: {}", self.message, self.missing_vars.join(", "))
    }
}

impl std::error::Error for SubstitutionError {}

/// Replaces environment variable references in configuration strings with audit logging.
///
/// Supports `$VAR_NAME` syntax for environment variable substitution.
/// - If a variable exists, it's replaced with its value
/// - If a variable doesn't exist, it's replaced with an empty string and a warning is logged
/// - `$$` is treated as an escaped dollar sign and becomes `$`
///
/// # Arguments
/// * `input` - The configuration string that may contain environment variable references
///
/// # Returns
/// A tuple of (substituted_string, audit_info) with audit_info containing substituted and missing vars
pub fn substitute_env_vars(input: &str) -> (String, SubstitutionAudit) {
    let mut audit = SubstitutionAudit::new();
    
    // Handle escaped dollar signs ($$) first by temporarily replacing them
    let placeholder = "\x00ESCAPED_DOLLAR\x00";
    let escaped_replaced = input.replace("$$", placeholder);

    // Replace environment variable references
    let result = ENV_VAR_REGEX.replace_all(&escaped_replaced, |caps: &regex::Captures| {
        let var_name = &caps[1];
        match env::var(var_name) {
            Ok(value) => {
                audit.substituted_vars.insert(var_name.to_string(), true);
                audit.substituted_values.insert(var_name.to_string(), value.clone());
                value
            }
            Err(_) => {
                audit.missing_vars.push(var_name.to_string());
                tracing::warn!(
                    "Environment variable '{}' not found in configuration, replacing with empty string",
                    var_name
                );
                String::new()
            }
        }
    });

    // Restore escaped dollar signs
    let final_result = result.replace(placeholder, "$");
    
    // Log audit information
    if !audit.substituted_vars.is_empty() {
        let var_names: Vec<&str> = audit.substituted_vars.keys().map(|s| s.as_str()).collect();
        tracing::info!(
            "Substituted {} environment variable{}: {}",
            var_names.len(),
            if var_names.len() == 1 { "" } else { "s" },
            var_names.join(", ")
        );
    }
    
    if !audit.missing_vars.is_empty() {
        tracing::warn!(
            "{} environment variable{} not found: {}",
            audit.missing_vars.len(),
            if audit.missing_vars.len() == 1 { "" } else { "s" },
            audit.missing_vars.join(", ")
        );
    }
    
    (final_result, audit)
}

/// Audit information about environment variable substitutions
#[derive(Debug, Clone, Default)]
pub struct SubstitutionAudit {
    /// Variables that were successfully substituted (name -> was_set)
    pub substituted_vars: HashMap<String, bool>,
    /// Substituted variable values (kept for validation; not logged)
    pub substituted_values: HashMap<String, String>,
    /// Variables that were referenced but not found
    pub missing_vars: Vec<String>,
}

impl SubstitutionAudit {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns all referenced variable names (both substituted and missing)
    pub fn all_referenced_vars(&self) -> Vec<String> {
        let mut all = self.substituted_vars.keys().cloned().collect::<Vec<_>>();
        all.extend(self.missing_vars.iter().cloned());
        all.sort();
        all.dedup();
        all
    }

    /// Returns true if any variables were missing
    pub fn has_missing_vars(&self) -> bool {
        !self.missing_vars.is_empty()
    }
}

/// Extracts all environment variable references from a configuration string
pub fn extract_variable_references(input: &str) -> Vec<String> {
    let mut vars = Vec::new();
    for caps in ENV_VAR_REGEX.captures_iter(input) {
        if let Some(var_name) = caps.get(1) {
            let name = var_name.as_str().to_string();
            if !vars.contains(&name) {
                vars.push(name);
            }
        }
    }
    vars.sort();
    vars
}

/// Validates that all required environment variables are present before substitution
pub fn substitute_env_vars_with_validation(
    input: &str,
    required_vars: &[&str],
) -> SubstitutionResult<(String, SubstitutionAudit)> {
    // Check that all required variables exist
    let mut missing = Vec::new();
    for var_name in required_vars {
        if env::var(var_name).is_err() {
            missing.push(var_name.to_string());
        }
    }

    if !missing.is_empty() {
        return Err(SubstitutionError {
            missing_vars: missing.clone(),
            message: format!(
                "Configuration validation failed: Missing required environment variables"
            ),
        });
    }

    // All required vars present, proceed with substitution
    Ok(substitute_env_vars(input))
}

/// Validation warnings for substituted values
#[derive(Debug, Clone, Default)]
pub struct ValidationWarnings {
    pub warnings: Vec<String>,
}

/// Validates substituted environment variable values for potentially unsafe content
pub fn validate_substituted_values(audit: &SubstitutionAudit) -> ValidationWarnings {
    let mut warnings = ValidationWarnings::default();

    for (var_name, value) in &audit.substituted_values {
        // Check for newlines
        if value.contains('\n') {
            warnings.warnings.push(format!(
                "Environment variable '{}' contains newlines, may break TOML parsing",
                var_name
            ));
        }

        // Check for unescaped quotes (basic check)
        if value.contains('"') && !value.contains("\\\"") {
            warnings.warnings.push(format!(
                "Environment variable '{}' contains unescaped quotes, verify TOML string formatting",
                var_name
            ));
        }

        // Check for control characters
        if value.chars().any(|c| c.is_control() && !c.is_whitespace()) {
            warnings.warnings.push(format!(
                "Environment variable '{}' contains control characters",
                var_name
            ));
        }

        // Check for shell metacharacters (warning only, not an error)
        if value.contains('$')
            || value.contains('`')
            || value.contains(';')
            || value.contains('|')
            || value.contains('&')
        {
            warnings.warnings.push(format!(
                "Environment variable '{}' contains shell metacharacters, verify this is intentional",
                var_name
            ));
        }
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_substitution() {
        env::set_var("TEST_VAR", "test_value");
        let input = "key = \"$TEST_VAR\"";
        let (result, _audit) = substitute_env_vars(input);
        assert_eq!(result, "key = \"test_value\"");
    }

    #[test]
    fn test_missing_variable() {
        env::remove_var("DEFINITELY_NOT_SET_VAR_XYZ");
        let input = "key = \"$DEFINITELY_NOT_SET_VAR_XYZ\"";
        let (result, audit) = substitute_env_vars(input);
        assert_eq!(result, "key = \"\"");
        assert_eq!(audit.missing_vars.len(), 1);
    }

    #[test]
    fn test_multiple_variables() {
        env::set_var("VAR1", "value1");
        env::set_var("VAR2", "value2");
        let input = "key = \"$VAR1 and $VAR2\"";
        let (result, audit) = substitute_env_vars(input);
        assert_eq!(result, "key = \"value1 and value2\"");
        assert_eq!(audit.substituted_vars.len(), 2);
    }

    #[test]
    fn test_escaped_dollar_sign() {
        let input = "key = \"price is $$100\"";
        let (result, _audit) = substitute_env_vars(input);
        assert_eq!(result, "key = \"price is $100\"");
    }

    #[test]
    fn test_escaped_and_variable() {
        env::set_var("AMOUNT", "50");
        let input = "key = \"$$ costs $$100, discount $$20, new price $AMOUNT\"";
        let (result, _audit) = substitute_env_vars(input);
        assert_eq!(result, "key = \"$ costs $100, discount $20, new price 50\"");
    }

    #[test]
    fn test_variable_in_url() {
        env::set_var("HOST", "example.com");
        env::set_var("PORT", "8080");
        let input = "url = \"http://$HOST:$PORT/api\"";
        let (result, _audit) = substitute_env_vars(input);
        assert_eq!(result, "url = \"http://example.com:8080/api\"");
    }

    #[test]
    fn test_variable_at_start() {
        env::set_var("PREFIX", "pre");
        let input = "$PREFIX_something";
        let (result, _audit) = substitute_env_vars(input);
        assert_eq!(result, "");
    }

    #[test]
    fn test_variable_at_end() {
        env::set_var("SUFFIX", "suf");
        let input = "something_$SUFFIX";
        let (result, _audit) = substitute_env_vars(input);
        assert_eq!(result, "something_suf");
    }

    #[test]
    fn test_underscore_in_variable_name() {
        env::set_var("MY_LONG_VAR_NAME", "value");
        let input = "key = \"$MY_LONG_VAR_NAME\"";
        let (result, _audit) = substitute_env_vars(input);
        assert_eq!(result, "key = \"value\"");
    }

    #[test]
    fn test_no_variables() {
        let input = "key = \"static_value\"";
        let (result, audit) = substitute_env_vars(input);
        assert_eq!(result, "key = \"static_value\"");
        assert_eq!(audit.substituted_vars.len(), 0);
        assert_eq!(audit.missing_vars.len(), 0);
    }

    #[test]
    fn test_invalid_variable_names_not_matched() {
        let input = "key = \"$123invalid\"";
        let (result, _audit) = substitute_env_vars(input);
        assert_eq!(result, "key = \"$123invalid\"");
    }

    #[test]
    fn test_hyphen_in_variable_name() {
        let input = "key = \"$my-var\"";
        let (result, _audit) = substitute_env_vars(input);
        assert_eq!(result, "key = \"-var\"");
    }

    #[test]
    fn test_mixed_valid_and_invalid() {
        env::set_var("VALID", "works");
        let input = "key = \"$VALID-$123-$INVALID_NAME\"";
        let (result, _audit) = substitute_env_vars(input);
        assert_eq!(result, "key = \"works-$123-\"");
    }

    #[test]
    fn test_extract_variable_references() {
        let input = "url = \"$HOST:$PORT/path\"; key = \"$KEY_AGAIN\"";
        let vars = extract_variable_references(input);
        assert_eq!(vars, vec!["HOST", "KEY_AGAIN", "PORT"]);
    }

    #[test]
    fn test_required_vars_validation_success() {
        env::set_var("REQ_VAR1", "value1");
        env::set_var("REQ_VAR2", "value2");
        let input = "key = \"$REQ_VAR1 and $REQ_VAR2\"";
        let result = substitute_env_vars_with_validation(input, &["REQ_VAR1", "REQ_VAR2"]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_required_vars_validation_missing() {
        env::remove_var("MISSING_REQ_VAR_XYZ");
        let input = "key = \"$MISSING_REQ_VAR_XYZ\"";
        let result = substitute_env_vars_with_validation(input, &["MISSING_REQ_VAR_XYZ"]);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.missing_vars.contains(&"MISSING_REQ_VAR_XYZ".to_string()));
        }
    }

    #[test]
    fn test_validate_substituted_values_with_newline() {
        env::set_var("BAD_VAR", "line1\nline2");
        let input = "config = \"$BAD_VAR\"";
        let (_result, audit) = substitute_env_vars(input);
        let warnings = validate_substituted_values(&audit);
        assert!(!warnings.warnings.is_empty());
        assert!(warnings.warnings.iter().any(|w| w.contains("newlines")));
    }

    #[test]
    fn test_validate_substituted_values_with_unescaped_quote() {
        env::set_var("BAD_VAR", "value\"with\"quotes");
        let input = "config = \"$BAD_VAR\"";
        let (_result, audit) = substitute_env_vars(input);
        let warnings = validate_substituted_values(&audit);
        assert!(!warnings.warnings.is_empty());
    }

    #[test]
    fn test_validate_substituted_values_with_shell_metachar() {
        env::set_var("SHELL_VAR", "value;command");
        let input = "config = \"$SHELL_VAR\"";
        let (_result, audit) = substitute_env_vars(input);
        let warnings = validate_substituted_values(&audit);
        assert!(!warnings.warnings.is_empty());
        assert!(warnings.warnings.iter().any(|w| w.contains("shell metacharacters")));
    }

    #[test]
    fn test_validate_substituted_values_safe() {
        env::set_var("GOOD_VAR", "safe_value_123");
        let input = "config = \"$GOOD_VAR\"";
        let (_result, audit) = substitute_env_vars(input);
        let warnings = validate_substituted_values(&audit);
        assert!(warnings.warnings.is_empty());
    }
}
