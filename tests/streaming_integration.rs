use harmony::config::config::Config;
use harmony::pipeline::PipelineExecutor;

/// Test that PipelineExecutor correctly detects streaming pipelines
#[tokio::test]
async fn test_streaming_pipeline_detection() {
    let config_toml = r#"
        [proxy]
        id = "streaming-detection-test"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.streaming_pipeline]
        networks = ["default"]
        endpoints = ["http_test"]
        backends = ["streaming_backend"]
        middleware = []

        [pipelines.regular_pipeline]
        networks = ["default"]
        endpoints = ["http_regular"]
        backends = ["echo_backend"]
        middleware = []

        [endpoints.http_test]
        service = "http"
        [endpoints.http_test.options]
        path_prefix = "/test"

        # Two pipelines on one network cannot share an endpoint: both would
        # claim the same paths, which fails config load (AURA-2443).
        [endpoints.http_regular]
        service = "http"
        [endpoints.http_regular.options]
        path_prefix = "/regular"

        [backends.streaming_backend]
        service = "dicom_scu_streaming"
        [backends.streaming_backend.options]
        aet = "TEST"
        host = "localhost"
        port = 4242

        [backends.echo_backend]
        service = "echo"

        [services.http]
        module = ""
        [services.echo]
        module = ""
        [services.dicom_scu_streaming]
        module = ""
    "#;

    let config: Config = toml::from_str(config_toml).unwrap();
    config.validate().unwrap();

    // Test streaming pipeline detection
    let streaming_pipeline = config.pipelines.get("streaming_pipeline").unwrap();
    assert!(PipelineExecutor::is_streaming_pipeline(
        streaming_pipeline,
        &config
    ));

    // Test non-streaming pipeline detection
    let regular_pipeline = config.pipelines.get("regular_pipeline").unwrap();
    assert!(!PipelineExecutor::is_streaming_pipeline(
        regular_pipeline,
        &config
    ));
}

/// Test HTTP streaming backend service type
#[tokio::test]
async fn test_http_streaming_backend_detection() {
    let config_toml = r#"
        [proxy]
        id = "http-backend-test"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.http_stream]
        networks = ["default"]
        endpoints = ["http_test"]
        backends = ["http_backend"]
        middleware = []

        [endpoints.http_test]
        service = "http"
        [endpoints.http_test.options]
        path_prefix = "/test"

        [backends.http_backend]
        service = "http_streaming_backend"
        [backends.http_backend.options]
        base_url = "http://localhost:9999"

        [services.http]
        module = ""
        [services.http_streaming_backend]
        module = ""
    "#;

    let config: Config = toml::from_str(config_toml).unwrap();
    config.validate().unwrap();

    // Verify pipeline is detected as streaming
    let pipeline = config.pipelines.get("http_stream").unwrap();
    assert!(PipelineExecutor::is_streaming_pipeline(pipeline, &config));
}

/// Test StreamFormat parsing from config
#[tokio::test]
async fn test_stream_format_from_config() {
    use harmony::models::services::types::http_streaming::StreamFormat;

    // Test valid formats
    assert_eq!(StreamFormat::from_config("sse"), StreamFormat::Sse);
    assert_eq!(StreamFormat::from_config("SSE"), StreamFormat::Sse);
    assert_eq!(StreamFormat::from_config("ndjson"), StreamFormat::Ndjson);
    assert_eq!(StreamFormat::from_config("NDJSON"), StreamFormat::Ndjson);
    assert_eq!(
        StreamFormat::from_config("multipart"),
        StreamFormat::Multipart
    );

    // Test default fallback
    assert_eq!(StreamFormat::from_config("invalid"), StreamFormat::Ndjson);
}

/// Test StreamFormat detection from Accept header
#[tokio::test]
async fn test_stream_format_from_accept_header() {
    use harmony::models::services::types::http_streaming::StreamFormat;

    // Test SSE detection
    assert_eq!(
        StreamFormat::from_accept_header("text/event-stream"),
        StreamFormat::Sse
    );

    // Test NDJSON detection
    assert_eq!(
        StreamFormat::from_accept_header("application/x-ndjson"),
        StreamFormat::Ndjson
    );

    // Test Multipart detection
    assert_eq!(
        StreamFormat::from_accept_header("multipart/x-mixed-replace"),
        StreamFormat::Multipart
    );

    // Test default fallback
    assert_eq!(
        StreamFormat::from_accept_header("application/json"),
        StreamFormat::Ndjson
    );
}

/// Test DICOM SCU streaming backend configuration
#[tokio::test]
async fn test_dicom_scu_streaming_backend_config() {
    let config_toml = r#"
        [proxy]
        id = "dicom-streaming-test"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.dicom_stream]
        networks = ["default"]
        endpoints = ["dicom_scp"]
        backends = ["pacs_streaming"]
        middleware = []

        [endpoints.dicom_scp]
        service = "dicom_scp"
        [endpoints.dicom_scp.options]
        local_aet = "TEST_SCP"
        port = 11112

        [backends.pacs_streaming]
        service = "dicom_scu_streaming"
        [backends.pacs_streaming.options]
        aet = "ORTHANC"
        host = "localhost"
        port = 4242
        local_aet = "TEST_SCU"

        [services.dicom_scp]
        module = ""
        [services.dicom_scu_streaming]
        module = ""
    "#;

    let config: Config = toml::from_str(config_toml).unwrap();
    config.validate().unwrap();

    // Verify pipeline is detected as streaming
    let pipeline = config.pipelines.get("dicom_stream").unwrap();
    assert!(PipelineExecutor::is_streaming_pipeline(pipeline, &config));

    // Verify backend configuration
    let backend = config.backends.get("pacs_streaming").unwrap();
    assert_eq!(backend.service, "dicom_scu_streaming");
    assert_eq!(
        backend.options.as_ref().unwrap().get("aet").unwrap(),
        "ORTHANC"
    );
}

/// Test HTTP streaming endpoint configuration
#[tokio::test]
async fn test_http_streaming_endpoint_config() {
    let config_toml = r#"
        [proxy]
        id = "http-stream-endpoint-test"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.http_stream]
        networks = ["default"]
        endpoints = ["http_streaming"]
        backends = ["pacs_streaming"]
        middleware = []

        [endpoints.http_streaming]
        service = "http_streaming"
        [endpoints.http_streaming.options]
        default_format = "ndjson"
        buffer_size = 10

        [backends.pacs_streaming]
        service = "dicom_scu_streaming"
        [backends.pacs_streaming.options]
        aet = "ORTHANC"
        host = "localhost"
        port = 4242

        [services.http_streaming]
        module = ""
        [services.dicom_scu_streaming]
        module = ""
    "#;

    let config: Config = toml::from_str(config_toml).unwrap();
    config.validate().unwrap();

    // Verify endpoint configuration
    let endpoint = config.endpoints.get("http_streaming").unwrap();
    assert_eq!(endpoint.service, "http_streaming");

    let options = endpoint.options.as_ref().unwrap();
    assert_eq!(options.get("default_format").unwrap(), "ndjson");
    assert_eq!(options.get("buffer_size").unwrap(), 10);
}

/// Test bidirectional streaming configuration
#[tokio::test]
async fn test_bidirectional_streaming_config() {
    let config_toml = r#"
        [proxy]
        id = "bidirectional-test"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.sender]
        enable_wireguard = false
        interface = "wg0"
        [network.sender.http]
        bind_address = "127.0.0.1"
        bind_port = 8443

        [network.receiver]
        enable_wireguard = false
        interface = "wg0"
        [network.receiver.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        # Sender side
        [pipelines.sender_pipeline]
        networks = ["sender"]
        endpoints = ["dicom_scp"]
        backends = ["local_pacs"]
        middleware = []

        [endpoints.dicom_scp]
        service = "dicom_scp"
        [endpoints.dicom_scp.options]
        local_aet = "SENDER_SCP"
        port = 11112

        [backends.local_pacs]
        service = "dicom_scu_streaming"
        [backends.local_pacs.options]
        aet = "LOCAL_PACS"
        host = "localhost"
        port = 4242

        # Receiver side
        [pipelines.receiver_pipeline]
        networks = ["receiver"]
        endpoints = ["http_api"]
        backends = ["remote_stream"]
        middleware = []

        [endpoints.http_api]
        service = "http"
        [endpoints.http_api.options]
        path_prefix = "/api"

        [backends.remote_stream]
        service = "http_streaming_backend"
        [backends.remote_stream.options]
        base_url = "https://sender-proxy:8443"
        format = "ndjson"

        [services.dicom_scp]
        module = ""
        [services.dicom_scu_streaming]
        module = ""
        [services.http]
        module = ""
        [services.http_streaming_backend]
        module = ""
    "#;

    let config: Config = toml::from_str(config_toml).unwrap();
    config.validate().unwrap();

    // Verify both pipelines
    assert!(config.pipelines.contains_key("sender_pipeline"));
    assert!(config.pipelines.contains_key("receiver_pipeline"));

    // Verify both are streaming
    let sender = config.pipelines.get("sender_pipeline").unwrap();
    assert!(PipelineExecutor::is_streaming_pipeline(sender, &config));

    let receiver = config.pipelines.get("receiver_pipeline").unwrap();
    assert!(PipelineExecutor::is_streaming_pipeline(receiver, &config));
}

/// Test backward compatibility with non-streaming pipelines
#[tokio::test]
async fn test_backward_compatibility() {
    let config_toml = r#"
        [proxy]
        id = "compat-test"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.regular]
        networks = ["default"]
        endpoints = ["http_test"]
        backends = ["echo_backend"]
        middleware = []

        [endpoints.http_test]
        service = "http"
        [endpoints.http_test.options]
        path_prefix = "/test"

        [backends.echo_backend]
        service = "echo"

        [services.http]
        module = ""
        [services.echo]
        module = ""
    "#;

    let config: Config = toml::from_str(config_toml).unwrap();
    config.validate().unwrap();

    // Verify non-streaming pipeline
    let pipeline = config.pipelines.get("regular").unwrap();
    assert!(!PipelineExecutor::is_streaming_pipeline(pipeline, &config));
}

/// Regression: a DIMSE SCP endpoint fronted by http_streaming_backend must be
/// classified as a streaming pipeline. C-STORE previously ignored this and
/// invoked the non-streaming executor, which the streaming backend refused.
/// The store handler now routes through `execute_streaming` when this returns
/// true, so this assertion pins the contract the fix relies on.
#[tokio::test]
async fn test_cstore_dimse_with_http_streaming_backend_is_streaming() {
    let config_toml = r#"
        [proxy]
        id = "cstore-streaming-test"

        [storage]
        backend = "filesystem"
        [storage.options]
        path = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.store_pipeline]
        networks = ["default"]
        endpoints = ["dicom_scp"]
        backends = ["http_stream"]
        middleware = []

        [endpoints.dicom_scp]
        service = "dicom_scp"
        [endpoints.dicom_scp.options]
        local_aet = "STORE_SCP"
        port = 11112
        enable_store = true

        [backends.http_stream]
        service = "http_streaming_backend"
        [backends.http_stream.options]
        base_url = "http://localhost:9999"
        format = "ndjson"

        [services.dicom_scp]
        module = ""
        [services.http_streaming_backend]
        module = ""
    "#;

    let config: Config = toml::from_str(config_toml).unwrap();
    config.validate().unwrap();

    let pipeline = config.pipelines.get("store_pipeline").unwrap();
    assert!(
        PipelineExecutor::is_streaming_pipeline(pipeline, &config),
        "store pipeline with http_streaming_backend must classify as streaming"
    );
}
