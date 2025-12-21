use harmony::config::config::Config;
use harmony::config::resolution::resolve_references;
use harmony::models::backends::backends::Backend;
use harmony::models::connection::ConnectionConfig;
use harmony::models::endpoints::endpoint::Endpoint;
use harmony::models::peers::config::PeerConfig;
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
