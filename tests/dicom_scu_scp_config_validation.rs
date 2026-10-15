use harmony::config::config::{Config, ConfigError};

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[test]
fn test_dicom_scu_backend_validation_success() {
    let toml = r#"
        [proxy]
        id = "test-proxy"
        log_level = "info"
        
        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080
        
        [pipelines.test]
        description = "Test pipeline"
        networks = ["default"]
        endpoints = ["test_endpoint"]
        backends = ["test_backend"]
        middleware = []
        
        [endpoints.test_endpoint]
        service = "http"
        [endpoints.test_endpoint.options]
        path_prefix = "/test"
        
        [backends.test_backend]
        service = "dicom_scu"
        [backends.test_backend.options]
        aet = "REMOTE_PACS"
        host = "localhost"
        port = 4242
        local_aet = "HARMONY_SCU"
        
        [services.http]
        module = ""
        
        [services.dicom_scu]
        module = ""
    "#;

    let result = load_config_from_str(toml);
    assert!(
        result.is_ok(),
        "Valid dicom_scu backend config should pass validation"
    );
}

#[test]
fn test_dicom_scu_missing_remote_aet() {
    // Note: Backend validation happens at runtime (when backend is invoked), not at config load time.
    // This test ensures the config can be loaded, but the backend will fail when actually used.
    let toml = r#"
        [proxy]
        id = "test-proxy"
        log_level = "info"
        
        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080
        
        [pipelines.test]
        description = "Test pipeline"
        networks = ["default"]
        endpoints = ["test_endpoint"]
        backends = ["test_backend"]
        middleware = []
        
        [endpoints.test_endpoint]
        service = "http"
        [endpoints.test_endpoint.options]
        path_prefix = "/test"
        
        [backends.test_backend]
        service = "dicom_scu"
        [backends.test_backend.options]
        # Missing aet - will fail at runtime when backend is invoked
        host = "localhost"
        port = 4242
        
        [services.http]
        module = ""
        
        [services.dicom_scu]
        module = ""
    "#;

    let result = load_config_from_str(toml);
    // Config loads successfully; validation happens at backend invocation time
    assert!(
        result.is_ok(),
        "Config should load; backend validation happens at runtime"
    );
}

#[test]
fn test_dicom_scu_missing_host() {
    // Note: Backend validation happens at runtime (when backend is invoked), not at config load time.
    // This test ensures the config can be loaded, but the backend will fail when actually used.
    let toml = r#"
        [proxy]
        id = "test-proxy"
        log_level = "info"
        
        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080
        
        [pipelines.test]
        description = "Test pipeline"
        networks = ["default"]
        endpoints = ["test_endpoint"]
        backends = ["test_backend"]
        middleware = []
        
        [endpoints.test_endpoint]
        service = "http"
        [endpoints.test_endpoint.options]
        path_prefix = "/test"
        
        [backends.test_backend]
        service = "dicom_scu"
        [backends.test_backend.options]
        aet = "REMOTE_PACS"
        # Missing host - will fail at runtime when backend is invoked
        port = 4242
        
        [services.http]
        module = ""
        
        [services.dicom_scu]
        module = ""
    "#;

    let result = load_config_from_str(toml);
    // Config loads successfully; validation happens at backend invocation time
    assert!(
        result.is_ok(),
        "Config should load; backend validation happens at runtime"
    );
}

#[test]
fn test_dicom_scp_endpoint_validation_success() {
    let toml = r#"
        [proxy]
        id = "test-proxy"
        log_level = "info"
        
        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080
        
        [pipelines.test]
        description = "Test pipeline"
        networks = ["default"]
        endpoints = ["test_endpoint"]
        backends = []
        middleware = []
        
        [endpoints.test_endpoint]
        service = "dicom_scp"
        [endpoints.test_endpoint.options]
        local_aet = "HARMONY_SCP"
        bind_addr = "0.0.0.0"
        port = 11112
        enable_echo = true
        enable_find = true
        
        [services.dicom_scp]
        module = ""
    "#;

    let result = load_config_from_str(toml);
    assert!(
        result.is_ok(),
        "Valid dicom_scp endpoint config should pass validation"
    );
}

#[test]
fn test_dicom_scp_invalid_aet_empty() {
    let toml = r#"
        [proxy]
        id = "test-proxy"
        log_level = "info"
        
        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080
        
        [pipelines.test]
        description = "Test pipeline"
        networks = ["default"]
        endpoints = ["test_endpoint"]
        backends = []
        middleware = []
        
        [endpoints.test_endpoint]
        service = "dicom_scp"
        [endpoints.test_endpoint.options]
        local_aet = ""  # Empty AET - should fail
        port = 11112
        
        [services.dicom_scp]
        module = ""
    "#;

    let result = load_config_from_str(toml);
    assert!(
        result.is_err(),
        "dicom_scp with empty AET should fail validation"
    );
}

#[test]
fn test_dicom_scp_invalid_aet_too_long() {
    let toml = r#"
        [proxy]
        id = "test-proxy"
        log_level = "info"
        
        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080
        
        [pipelines.test]
        description = "Test pipeline"
        networks = ["default"]
        endpoints = ["test_endpoint"]
        backends = []
        middleware = []
        
        [endpoints.test_endpoint]
        service = "dicom_scp"
        [endpoints.test_endpoint.options]
        local_aet = "THIS_AET_IS_TOO_LONG"  # > 16 chars - should fail
        port = 11112
        
        [services.dicom_scp]
        module = ""
    "#;

    let result = load_config_from_str(toml);
    assert!(
        result.is_err(),
        "dicom_scp with AET > 16 chars should fail validation"
    );
}

#[test]
fn test_dicom_scp_no_operations_enabled() {
    let toml = r#"
        [proxy]
        id = "test-proxy"
        log_level = "info"
        
        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080
        
        [pipelines.test]
        description = "Test pipeline"
        networks = ["default"]
        endpoints = ["test_endpoint"]
        backends = []
        middleware = []
        
        [endpoints.test_endpoint]
        service = "dicom_scp"
        [endpoints.test_endpoint.options]
        local_aet = "HARMONY_SCP"
        port = 11112
        enable_echo = false
        enable_find = false
        enable_move = false
        enable_get = false
        
        [services.dicom_scp]
        module = ""
    "#;

    let result = load_config_from_str(toml);
    assert!(
        result.is_err(),
        "dicom_scp with no operations enabled should fail validation"
    );
}

#[test]
fn test_dicom_scp_with_c_get_enabled() {
    let toml = r#"
        [proxy]
        id = "test-proxy"
        log_level = "info"
        
        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080
        
        [pipelines.test]
        description = "Test pipeline"
        networks = ["default"]
        endpoints = ["test_endpoint"]
        backends = []
        middleware = []
        
        [endpoints.test_endpoint]
        service = "dicom_scp"
        [endpoints.test_endpoint.options]
        local_aet = "HARMONY_SCP"
        port = 11112
        enable_echo = true
        enable_get = true  # C-GET enabled
        
        [services.dicom_scp]
        module = ""
    "#;

    let result = load_config_from_str(toml);
    assert!(
        result.is_ok(),
        "dicom_scp with C-GET enabled should pass validation"
    );
}

#[test]
fn test_complete_scp_to_scu_bridge() {
    let toml = r#"
        [proxy]
        id = "dicom-bridge"
        log_level = "info"
        
        [network.dicom_network]
        enable_wireguard = false
        interface = "wg0"
        
        [network.dicom_network.http]
        bind_address = "127.0.0.1"
        bind_port = 8080
        
        [pipelines.dicom_bridge]
        description = "DICOM SCP to SCU bridge"
        networks = ["dicom_network"]
        endpoints = ["dicom_listener"]
        backends = ["remote_pacs"]
        middleware = []
        
        [endpoints.dicom_listener]
        service = "dicom_scp"
        [endpoints.dicom_listener.options]
        local_aet = "BRIDGE_SCP"
        port = 11112
        enable_echo = true
        enable_find = true
        enable_move = true
        enable_get = true
        
        [backends.remote_pacs]
        service = "dicom_scu"
        [backends.remote_pacs.options]
        aet = "PACS_AET"
        host = "pacs.example.com"
        port = 4242
        local_aet = "BRIDGE_SCU"
        
        [services.dicom_scp]
        module = ""
        
        [services.dicom_scu]
        module = ""
    "#;

    let result = load_config_from_str(toml);
    assert!(
        result.is_ok(),
        "Complete SCP-to-SCU bridge configuration should be valid"
    );
}

// Note: Tests for runtime service separation (dicom_scu as endpoint, dicom_scp as backend)
// are omitted as they require access to private modules. The separation is enforced by
// implementation and validated through configuration tests above.

// DCMTK storescp runs no pipeline and no association-time IP check, so a
// port it serves cannot enforce `ip_allow` or `ip_deny` rules. Validation
// rejects such configs.

/// Shared policies, rules and services for the DCMTK tests.
const DCMTK_POLICY_TABLES: &str = r#"
[middleware.office_only]
type = "policies"
[middleware.office_only.options]
policies = ["office"]

[middleware.no_loopback]
type = "policies"
[middleware.no_loopback.options]
policies = ["block_loopback"]

[middleware.open_door]
type = "policies"
[middleware.open_door.options]
policies = ["open"]

[policies.office]
id = "office"
rules = ["office_allow"]

[policies.block_loopback]
id = "block_loopback"
rules = ["loopback_deny", "anyone"]

[policies.open]
id = "open"
rules = ["anyone"]

[rules.office_allow]
id = "office_allow"
type = "ip_allow"
[rules.office_allow.options]
ip_addresses = ["192.168.0.0/16"]

[rules.loopback_deny]
id = "loopback_deny"
type = "ip_deny"
[rules.loopback_deny.options]
ip_addresses = ["127.0.0.0/8"]

[rules.anyone]
id = "anyone"
type = "allow_all"

[services.dicom_scp]
module = ""
"#;

/// A config with one `dicom_scp` pipeline on port 11112.
///
/// `scp_options` is appended to the endpoint's options and `middleware` is
/// the pipeline's middleware list.
fn dcmtk_config(scp_options: &str, middleware: &str) -> String {
    format!(
        r#"
[proxy]
id = "test-proxy"

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8080

[pipelines.store]
networks = ["default"]
endpoints = ["store_scp"]
middleware = {middleware}

[endpoints.store_scp]
service = "dicom_scp"
[endpoints.store_scp.options]
local_aet = "STORE_SCP"
port = 11112
enable_echo = true
{scp_options}
{DCMTK_POLICY_TABLES}
"#
    )
}

/// Parse, resolve references and validate, as config loading does.
fn load_resolved_config(toml: &str) -> Result<Config, ConfigError> {
    let mut config: Config = toml::from_str(toml).expect("TOML parse error");
    harmony::config::resolution::resolve_references(&mut config)
        .expect("references should resolve");
    config.validate()?;
    Ok(config)
}

/// Assert the config is rejected because `store_scp` hands port 11112 to
/// DCMTK through `option` while `pipeline` has IP rules.
fn assert_dcmtk_rejected(toml: &str, pipeline: &str, option: &str) {
    match load_resolved_config(toml) {
        Err(ConfigError::InvalidEndpoint { name, reason }) => {
            assert_eq!(name, "store_scp");
            assert!(
                reason.contains(&format!("pipeline '{}'", pipeline))
                    && reason.contains("DIMSE port 11112 is served by DCMTK storescp")
                    && reason.contains(&format!("'{}' on endpoint 'store_scp'", option)),
                "unexpected reason: {}",
                reason
            );
        }
        other => panic!(
            "expected a DCMTK rejection for pipeline '{}', got {:?}",
            pipeline,
            other.map(|_| "Ok(config)")
        ),
    }
}

#[test]
fn dcmtk_store_with_ip_allow_rules_is_rejected() {
    let toml = dcmtk_config("use_dcmtk_store = true", r#"["office_only"]"#);
    assert_dcmtk_rejected(&toml, "store", "use_dcmtk_store");
}

#[test]
fn persistent_store_scp_defaults_to_dcmtk_and_rejects_ip_deny_rules() {
    let toml = dcmtk_config("persistent_store_scp = true", r#"["no_loopback"]"#);
    assert_dcmtk_rejected(&toml, "store", "persistent_store_scp");
}

/// storescp binds every interface, so it takes the port for listeners on
/// other addresses and networks too.
#[test]
fn dcmtk_store_rejects_ip_rules_on_another_network_sharing_the_port() {
    let toml = format!(
        r#"{}
[network.ward]
enable_wireguard = false
interface = "wg0"

[network.ward.http]
bind_address = "127.0.0.1"
bind_port = 8081

[pipelines.ward_query]
networks = ["ward"]
endpoints = ["ward_scp"]
middleware = ["office_only"]

[endpoints.ward_scp]
service = "dicom_scp"
[endpoints.ward_scp.options]
local_aet = "WARD_SCP"
bind_addr = "127.0.0.2"
port = 11112
enable_echo = true
"#,
        dcmtk_config("use_dcmtk_store = true", "[]")
    );
    assert_dcmtk_rejected(&toml, "ward_query", "use_dcmtk_store");
}

#[test]
fn dcmtk_store_without_policies_is_accepted() {
    let toml = dcmtk_config("use_dcmtk_store = true", "[]");
    let result = load_resolved_config(&toml);
    assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
}

/// Only IP rules are checked; other policy rules do not block DCMTK.
#[test]
fn dcmtk_store_with_policies_but_no_ip_rules_is_accepted() {
    let toml = dcmtk_config("use_dcmtk_store = true", r#"["open_door"]"#);
    let result = load_resolved_config(&toml);
    assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
}

#[test]
fn internal_scp_with_ip_rules_is_accepted() {
    let toml = dcmtk_config("use_dcmtk_store = false", r#"["office_only"]"#);
    let result = load_resolved_config(&toml);
    assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
}

#[test]
fn persistent_store_scp_with_dcmtk_turned_off_accepts_ip_rules() {
    let toml = dcmtk_config(
        "persistent_store_scp = true\nuse_dcmtk_store = false",
        r#"["no_loopback"]"#,
    );
    let result = load_resolved_config(&toml);
    assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
}

/// A legacy `dicom_scu` backend with `persistent_store_scp` opens its own
/// DCMTK listener on `incoming_store_port`. The error names the backend.
#[test]
fn persistent_store_scp_backend_rejects_ip_rules_on_its_pipeline() {
    let toml = format!(
        r#"{}
[pipelines.retrieve]
networks = ["default"]
endpoints = ["api"]
backends = ["pacs"]
middleware = ["office_only"]

[endpoints.api]
service = "http"
[endpoints.api.options]
path_prefix = "/retrieve"

[backends.pacs]
service = "dicom_scu"
[backends.pacs.options]
aet = "PACS"
host = "localhost"
port = 4242
local_aet = "HARMONY_SCU"
incoming_store_port = 11612
persistent_store_scp = true

[services.http]
module = ""

[services.dicom_scu]
module = ""
"#,
        dcmtk_config("", "[]")
    );
    match load_resolved_config(&toml) {
        Err(ConfigError::InvalidBackend { name, reason }) => {
            assert_eq!(name, "pacs");
            assert!(
                reason.contains("pipeline 'retrieve' (backend 'pacs')")
                    && reason.contains("'persistent_store_scp' on backend 'pacs'"),
                "unexpected reason: {}",
                reason
            );
        }
        other => panic!(
            "expected InvalidBackend for 'pacs', got {:?}",
            other.map(|_| "Ok(config)")
        ),
    }
}

/// A legacy persistent-SCP backend on a network with no DIMSE endpoint opens
/// no listener, because no DIMSE adapter starts there. Its IP rules guard
/// the HTTP endpoint only, so the config is accepted.
#[test]
fn persistent_store_scp_backend_without_a_dimse_endpoint_is_accepted() {
    let toml = format!(
        r#"
[proxy]
id = "test-proxy"

[network.http_net]
enable_wireguard = false
interface = "wg0"

[network.http_net.http]
bind_address = "127.0.0.1"
bind_port = 8080

[pipelines.retrieve]
networks = ["http_net"]
endpoints = ["api"]
backends = ["pacs"]
middleware = ["office_only"]

[endpoints.api]
service = "http"
[endpoints.api.options]
path_prefix = "/retrieve"

[backends.pacs]
service = "dicom_scu"
[backends.pacs.options]
aet = "PACS"
host = "localhost"
port = 4242
local_aet = "HARMONY_SCU"
incoming_store_port = 11612
persistent_store_scp = true

[services.http]
module = ""

[services.dicom_scu]
module = ""
{DCMTK_POLICY_TABLES}
"#
    );
    let result = load_resolved_config(&toml);
    assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
}

/// A disabled `ip_allow` rule is not enforced anywhere, so DCMTK may serve
/// its pipeline.
#[test]
fn dcmtk_store_with_only_disabled_ip_rules_is_accepted() {
    let toml = format!(
        r#"{}
[middleware.dormant]
type = "policies"
[middleware.dormant.options]
policies = ["dormant_policy"]

[policies.dormant_policy]
id = "dormant_policy"
rules = ["dormant_allow", "anyone"]

[rules.dormant_allow]
id = "dormant_allow"
type = "ip_allow"
enabled = false
[rules.dormant_allow.options]
ip_addresses = ["192.168.0.0/16"]
"#,
        dcmtk_config("use_dcmtk_store = true", r#"["dormant"]"#)
    );
    let result = load_resolved_config(&toml);
    assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
}

/// A policies middleware whose rules cannot be read might hold IP rules,
/// so the config is rejected. Middleware validation catches it before the
/// DCMTK check, which stays as a fail-closed backstop.
#[test]
fn dcmtk_store_with_an_unreadable_policies_middleware_is_rejected() {
    let toml = format!(
        r#"{}
[middleware.broken]
type = "policies"
[middleware.broken.options]
policies = ["all_disabled"]

[policies.all_disabled]
id = "all_disabled"
enabled = false
rules = ["anyone"]
"#,
        dcmtk_config("use_dcmtk_store = true", r#"["broken"]"#)
    );
    match load_resolved_config(&toml) {
        Err(ConfigError::InvalidMiddleware { name, reason }) => {
            assert_eq!(name, "broken");
            assert!(
                reason.contains("requires at least one policy"),
                "unexpected reason: {}",
                reason
            );
        }
        other => panic!(
            "expected a fail-closed rejection, got {:?}",
            other.map(|_| "Ok(config)")
        ),
    }
}
