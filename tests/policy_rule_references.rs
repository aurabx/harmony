//! Undefined policy and rule ids must fail config load (AURA-2439).
//!
//! The policies middleware resolves ids per request. Without a load-time
//! check, a config with a bad id is accepted and every request returns 500.
//! These tests go through `Config::load`, the path used at startup, by
//! `--validate-config` and by hot reload.

use harmony::config::config::{Config, ConfigError};

/// Base config with two rules (`internal_networks`, `method_filter`) and a
/// pipeline that runs the `access` middleware instance. `extra` is appended
/// verbatim and must define `[middleware.access]`.
fn config_with(extra: &str) -> String {
    format!(
        r#"
[proxy]
id = "policy-ref-test"

[management]
enabled = false

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8080

[pipelines.core]
networks = ["default"]
endpoints = ["http_in"]
backends = ["api_backend"]
middleware = ["access"]

[endpoints.http_in]
service = "http"
[endpoints.http_in.options]
path_prefix = "/api"

[backends.api_backend]
service = "http"
[backends.api_backend.options]
host = "127.0.0.1"
port = 9

[services.http]
module = ""

[rules.internal_networks]
id = "internal_networks"
type = "ip_allow"
[rules.internal_networks.options]
ip_addresses = ["10.0.0.0/8"]

[rules.method_filter]
id = "method_filter"
type = "method"
[rules.method_filter.options]
mode = "allow"
methods = ["GET"]

{}
"#,
        extra
    )
}

/// Writes `toml` to a temp dir and loads it the way the binary does.
fn load_via_file(toml: &str) -> Result<Config, String> {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, toml).expect("write config");
    Config::load(path.to_str().expect("utf-8 path")).map_err(|e| match e {
        ConfigError::InvalidProxy { reason, .. } => reason,
        other => format!("{:?}", other),
    })
}

const ACCESS_MIDDLEWARE: &str = r#"
[middleware.access]
type = "policies"
[middleware.access.options]
policies = ["office"]
"#;

#[test]
fn policy_with_undefined_rule_id_fails_load() {
    let toml = config_with(&format!(
        r#"
[policies.office]
id = "office"
rules = ["internal_networks", "missing_rule"]
{}"#,
        ACCESS_MIDDLEWARE
    ));

    let err = load_via_file(&toml).expect_err("undefined rule id must fail load");

    assert!(
        err.contains(
            "Policy 'office' references unknown rule 'missing_rule'. \
             Available rules: internal_networks, method_filter"
        ),
        "unexpected error: {}",
        err
    );
}

#[test]
fn middleware_with_undefined_policy_id_fails_load() {
    let toml = config_with(
        r#"
[policies.office]
id = "office"
rules = ["internal_networks"]

[policies.vpn]
id = "vpn"
rules = ["method_filter"]

[middleware.access]
type = "policies"
[middleware.access.options]
policies = ["office", "missing_policy"]
"#,
    );

    let err = load_via_file(&toml).expect_err("undefined policy id must fail load");

    assert!(
        err.contains(
            "Middleware 'access' references unknown policy 'missing_policy'. \
             Available policies: office, vpn"
        ),
        "unexpected error: {}",
        err
    );
}

#[test]
fn every_undefined_policy_and_rule_id_is_reported() {
    let toml = config_with(
        r#"
[policies.office]
id = "office"
rules = ["zzz_rule", "internal_networks", "aaa_rule"]

[policies.vpn]
id = "vpn"
rules = ["vpn_rule"]

[middleware.access]
type = "policies"
[middleware.access.options]
policies = ["office", "ghost_policy"]

[middleware.second]
type = "policies"
[middleware.second.options]
policies = ["other_ghost"]
"#,
    );

    let err = load_via_file(&toml).expect_err("undefined ids must fail load");

    let expected = [
        "Middleware 'access' references unknown policy 'ghost_policy'. Available policies: office, vpn",
        "Middleware 'second' references unknown policy 'other_ghost'. Available policies: office, vpn",
        "Policy 'office' references unknown rule 'aaa_rule'. Available rules: internal_networks, method_filter",
        "Policy 'office' references unknown rule 'zzz_rule'. Available rules: internal_networks, method_filter",
        "Policy 'vpn' references unknown rule 'vpn_rule'. Available rules: internal_networks, method_filter",
    ]
    .join("\n");
    assert!(
        err.ends_with(&expected),
        "expected one sorted line per bad reference.\nexpected tail:\n{}\nactual:\n{}",
        expected,
        err
    );
}

#[test]
fn undefined_policy_id_lists_none_defined_when_no_policies_exist() {
    let toml = config_with(ACCESS_MIDDLEWARE);

    let err = load_via_file(&toml).expect_err("undefined policy id must fail load");

    assert!(
        err.contains(
            "Middleware 'access' references unknown policy 'office'. \
             Available policies: (none defined)"
        ),
        "unexpected error: {}",
        err
    );
}

/// A disabled policy is skipped at request time, so its bad rule id would
/// only surface when someone enables it. Reject it at load instead.
#[test]
fn disabled_policy_with_undefined_rule_id_fails_load() {
    let toml = config_with(
        r#"
[policies.office]
id = "office"
rules = ["internal_networks"]

[policies.parked]
id = "parked"
enabled = false
rules = ["missing_rule"]

[middleware.access]
type = "policies"
[middleware.access.options]
policies = ["office", "parked"]
"#,
    );

    let err = load_via_file(&toml).expect_err("disabled policy must still be checked");

    assert!(
        err.contains("Policy 'parked' references unknown rule 'missing_rule'."),
        "unexpected error: {}",
        err
    );
}

#[test]
fn valid_policies_config_loads() {
    let toml = config_with(
        r#"
[policies.office]
id = "office"
rules = ["internal_networks", "method_filter"]

[policies.parked]
id = "parked"
enabled = false
rules = ["method_filter"]

[middleware.access]
type = "policies"
[middleware.access.options]
policies = ["office", "parked"]
"#,
    );

    let config = load_via_file(&toml).expect("valid policies config must load");

    assert_eq!(config.policies.len(), 2);
    assert_eq!(config.rules.len(), 2);
}
