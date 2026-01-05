use harmony::config::config::Config;
use harmony::config::resolution::resolve_references;
use harmony::models::backends::backends::Backend;
use harmony::models::connection::ConnectionConfig;
use harmony::models::endpoints::endpoint::Endpoint;
use harmony::models::network::config::{NetworkConfig, TcpConfig};
use harmony::models::peers::config::PeerConfig;
use harmony::models::pipelines::config::{Pipeline, PipelineMiddleware};
use harmony::models::services::services::ServiceConfig;
use harmony::models::targets::config::TargetConfig;

#[test]
fn test_endpoint_peer_reference_resolution() {
    let mut config = Config::default();

    // Define a peer
    let peer_config = PeerConfig {
        id: Some("peer1".to_string()),
        name: Some("Peer 1".to_string()),
        connection: ConnectionConfig {
            host: "peer.example.com".to_string(),
            port: Some(11112),
            protocol: None,
            base_path: None,
            ca_cert_path: None,
        },
        protocol: Some("dicom".to_string()),
        description: None,
        enabled: true,
        authentication: None,
        tags: None,
        timeout_secs: 30,
        max_retries: 3,
    };
    config.peers.insert("peer1".to_string(), peer_config);

    // Define an endpoint referencing the peer
    let endpoint = Endpoint {
        service: "dicom_scp".to_string(),
        options: None,
        peer_ref: Some("peer1".to_string()),
        connection: None,
        authentication: None,
    };
    config.endpoints.insert("endpoint1".to_string(), endpoint);

    // Resolve references
    resolve_references(&mut config).expect("Resolution failed");

    // Check if endpoint resolved the connection
    let resolved_endpoint = config.endpoints.get("endpoint1").unwrap();
    let conn = resolved_endpoint.connection.as_ref().expect("Connection not resolved");
    
    assert_eq!(conn.host, "peer.example.com");
    assert_eq!(conn.port, Some(11112));
    // Peer's top-level protocol should be merged into connection
    assert_eq!(conn.protocol, Some("dicom".to_string()));
    
    // Check if options were injected
    let options = resolved_endpoint.options.as_ref().expect("Options not injected");
    assert!(options.contains_key("connection"));
}

#[test]
fn test_backend_target_reference_resolution() {
    let mut config = Config::default();

    // Define a target
    let target_config = TargetConfig {
        id: Some("target1".to_string()),
        name: Some("Target 1".to_string()),
        connection: ConnectionConfig {
            host: "api.example.com".to_string(),
            port: Some(443),
            protocol: None,
            base_path: Some("/v1".to_string()),
            ca_cert_path: None,
        },
        protocol: Some("https".to_string()),
        description: None,
        enabled: true,
        authentication: None,
        tags: None,
        timeout_secs: 60,
        max_retries: 5,
    };
    config.targets.insert("target1".to_string(), target_config);

    // Define a backend referencing the target
    let backend = Backend {
        service: "http".to_string(),
        options: None,
        target_ref: Some("target1".to_string()),
        connection: None,
        authentication: None,
        timeout_secs: None,
        max_retries: None,
    };
    config.backends.insert("backend1".to_string(), backend);

    // Resolve references
    resolve_references(&mut config).expect("Resolution failed");

    // Check if backend resolved the connection
    let resolved_backend = config.backends.get("backend1").unwrap();
    let conn = resolved_backend.connection.as_ref().expect("Connection not resolved");
    
    assert_eq!(conn.host, "api.example.com");
    assert_eq!(conn.port, Some(443));
    assert_eq!(conn.protocol, Some("https".to_string()));
    assert_eq!(conn.base_path, Some("/v1".to_string()));
    
    assert_eq!(resolved_backend.timeout_secs, Some(60));
    assert_eq!(resolved_backend.max_retries, Some(5));
    
    // Check if options were injected
    let options = resolved_backend.options.as_ref().expect("Options not injected");
    assert!(options.contains_key("connection"));
}

#[test]
fn test_override_precedence() {
    let mut config = Config::default();

    // Target with base settings
    let target_config = TargetConfig {
        id: Some("target1".to_string()),
        name: None,
        connection: ConnectionConfig {
            host: "base.com".to_string(),
            port: Some(80),
            protocol: None,
            base_path: None,
            ca_cert_path: None,
        },
        protocol: Some("http".to_string()),
        description: None,
        enabled: true,
        authentication: None,
        tags: None,
        timeout_secs: 30,
        max_retries: 3,
    };
    config.targets.insert("target1".to_string(), target_config);

    // Backend overriding host
    let backend = Backend {
        service: "http".to_string(),
        options: None,
        target_ref: Some("target1".to_string()),
        connection: Some(ConnectionConfig {
            host: "override.com".to_string(),
            port: None, // Inherit port
            protocol: None,
            base_path: None,
            ca_cert_path: None,
        }),
        authentication: None,
        timeout_secs: None,
        max_retries: None,
    };
    config.backends.insert("backend1".to_string(), backend);

    resolve_references(&mut config).expect("Resolution failed");

    let resolved = config.backends.get("backend1").unwrap();
    let conn = resolved.connection.as_ref().unwrap();
    
    assert_eq!(conn.host, "override.com");
    assert_eq!(conn.port, Some(80)); // Inherited
    assert_eq!(conn.protocol, Some("http".to_string())); // Inherited
}

#[test]
fn test_missing_target_returns_unresolved_backend() {
    let mut config = Config::default();

    // Define a backend referencing a non-existent target
    let backend = Backend {
        service: "http".to_string(),
        options: None,
        target_ref: Some("missing_target".to_string()),
        connection: None,
        authentication: None,
        timeout_secs: None,
        max_retries: None,
    };
    config.backends.insert("backend1".to_string(), backend);

    // Resolve references - should not panic, but return unresolved backend
    let unresolved = resolve_references(&mut config).expect("Resolution should not fail on missing target");

    // Verify that the backend was marked as unresolved
    assert!(unresolved.contains("backend1"), "Backend should be in unresolved set");
    assert_eq!(unresolved.len(), 1, "Only one backend should be unresolved");
}

#[test]
fn test_unresolved_backend_skipped_in_validation() {
    let mut config = Config::default();

    // Set up a network (required for pipelines)
    config.network.insert(
        "default".to_string(),
        NetworkConfig {
            enable_wireguard: false,
            interface: "wg0".to_string(),
            tcp_config: Some(TcpConfig {
                bind_address: "127.0.0.1".to_string(),
                bind_port: 8080,
                cert_path: None,
                key_path: None,
                force_https: false,
            }),
            http3: None,
        },
    );

    // Define a backend referencing a non-existent target
    let backend = Backend {
        service: "http".to_string(),
        options: None,
        target_ref: Some("missing_target".to_string()),
        connection: None,
        authentication: None,
        timeout_secs: None,
        max_retries: None,
    };
    config.backends.insert("unreachable_backend".to_string(), backend);

    // Create a pipeline referencing the unreachable backend
    config.pipelines.insert(
        "test_pipeline".to_string(),
        Pipeline {
            description: "Test pipeline with missing target".to_string(),
            networks: vec!["default".to_string()],
            endpoints: vec![],
            backends: vec!["unreachable_backend".to_string()],
            middleware: PipelineMiddleware::default(),
            ..Default::default()
        },
    );

    // Resolve references
    let unresolved = resolve_references(&mut config).expect("Resolution should succeed");
    config.unresolved_backends = unresolved;

    // Set required proxy and service defaults
    config.proxy.id = "test".to_string();
    config.proxy.jwks_cache_duration_hours = 24;  // Set valid duration
    config.services.insert("http".to_string(), ServiceConfig { id: None, module: "".to_string() });
    config.management.enabled = false;

    // Validation should not fail, but skip the unresolved backend and pipeline
    match config.validate() {
        Ok(()) => {}
        Err(e) => {
            panic!("Validation failed: {:?}", e);
        }
    }
}
