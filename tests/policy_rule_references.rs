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
    load_via_files(toml, None)
}

/// Writes `main_toml` as the main config and, when given, `pipelines_toml` as
/// `pipelines/policies.toml` next to it. `pipelines` is the default
/// `proxy.pipelines_path`, so `Config::load` merges that file in.
fn load_via_files(main_toml: &str, pipelines_toml: Option<&str>) -> Result<Config, String> {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, main_toml).expect("write config");
    if let Some(pipelines_toml) = pipelines_toml {
        let pipelines_dir = dir.path().join("pipelines");
        std::fs::create_dir(&pipelines_dir).expect("create pipelines dir");
        std::fs::write(pipelines_dir.join("policies.toml"), pipelines_toml)
            .expect("write pipelines config");
    }
    Config::load(path.to_str().expect("utf-8 path")).map_err(|e| match e {
        ConfigError::InvalidProxy { reason, .. } => reason,
        other => format!("{:?}", other),
    })
}

/// Asserts that `err` ends with exactly `lines`, one per line.
fn assert_error_tail(err: &str, lines: &[&str]) {
    let expected = lines.join("\n");
    assert!(
        err.ends_with(&format!(": {}", expected)),
        "unexpected error.\nexpected tail:\n{}\nactual:\n{}",
        expected,
        err
    );
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

    assert_error_tail(
        &err,
        &[
            "Policy 'office' references unknown rule 'missing_rule'",
            "Available rules: internal_networks, method_filter",
        ],
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

    assert_error_tail(
        &err,
        &[
            "Middleware 'access' references unknown policy 'missing_policy'",
            "Available policies: office, vpn",
        ],
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

    // One sorted line per bad id. Each list of defined ids appears once, so
    // the message does not grow with bad ids times defined ids.
    assert_error_tail(
        &err,
        &[
            "Middleware 'access' references unknown policy 'ghost_policy'",
            "Middleware 'second' references unknown policy 'other_ghost'",
            "Policy 'office' references unknown rule 'aaa_rule'",
            "Policy 'office' references unknown rule 'zzz_rule'",
            "Policy 'vpn' references unknown rule 'vpn_rule'",
            "Available rules: internal_networks, method_filter",
            "Available policies: office, vpn",
        ],
    );
    assert_eq!(err.matches("Available rules:").count(), 1);
    assert_eq!(err.matches("Available policies:").count(), 1);
}

#[test]
fn undefined_policy_id_lists_none_defined_when_no_policies_exist() {
    let toml = config_with(ACCESS_MIDDLEWARE);

    let err = load_via_file(&toml).expect_err("undefined policy id must fail load");

    assert_error_tail(
        &err,
        &[
            "Middleware 'access' references unknown policy 'office'",
            "Available policies: (none defined)",
        ],
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

    assert_error_tail(
        &err,
        &[
            "Policy 'parked' references unknown rule 'missing_rule'",
            "Available rules: internal_networks, method_filter",
        ],
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

/// Policies and rules often live in a `pipelines/` file while the middleware
/// instance sits in the main file. The check must see the merged config.
#[test]
fn policies_and_rules_from_pipelines_directory_load() {
    let main = config_with(ACCESS_MIDDLEWARE);
    let pipelines = r#"
[policies.office]
id = "office"
rules = ["internal_networks", "split_rule"]

[rules.split_rule]
id = "split_rule"
type = "allow_all"
"#;

    let config = load_via_files(&main, Some(pipelines)).expect("split policies config must load");

    assert!(config.policies.contains_key("office"));
    assert!(config.rules.contains_key("split_rule"));
}

#[test]
fn undefined_rule_id_in_pipelines_directory_fails_load() {
    let main = config_with(ACCESS_MIDDLEWARE);
    let pipelines = r#"
[policies.office]
id = "office"
rules = ["internal_networks", "missing_rule"]

[rules.split_rule]
id = "split_rule"
type = "allow_all"
"#;

    let err = load_via_files(&main, Some(pipelines))
        .expect_err("undefined rule id in a pipelines file must fail load");

    assert_error_tail(
        &err,
        &[
            "Policy 'office' references unknown rule 'missing_rule'",
            "Available rules: internal_networks, method_filter, split_rule",
        ],
    );
}

/// The middleware looks a policy up by its table key, not by its `id` field.
/// A reference to the `id` of a policy stored under another key is undefined.
#[test]
fn policy_referenced_by_id_field_instead_of_table_key_fails_load() {
    let toml = config_with(
        r#"
[policies.p_key]
id = "p_id"
rules = ["internal_networks"]

[middleware.access]
type = "policies"
[middleware.access.options]
policies = ["p_id"]
"#,
    );

    let err = load_via_file(&toml).expect_err("ids are matched by table key");

    assert_error_tail(
        &err,
        &[
            "Middleware 'access' references unknown policy 'p_id'",
            "Available policies: p_key",
        ],
    );
}

/// The middleware registry matches type names without regard to case, so the
/// check must cover an instance written as `type = "Policies"`.
#[test]
fn mixed_case_policies_type_with_undefined_policy_id_fails_load() {
    let toml = config_with(
        r#"
[policies.office]
id = "office"
rules = ["internal_networks"]

[middleware.access]
type = "Policies"
[middleware.access.options]
policies = ["missing_policy"]
"#,
    );

    let err = load_via_file(&toml).expect_err("mixed-case type must still be checked");

    assert_error_tail(
        &err,
        &[
            "Middleware 'access' references unknown policy 'missing_policy'",
            "Available policies: office",
        ],
    );
}
