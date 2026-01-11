# Adding/Configuring Backends in Harmony

## Reference Documentation
Full details: `docs/backends.md`

## Overview

Backends communicate with external targets (HTTP APIs, DICOM PACS, FHIR servers, etc.) within the pipeline.

```
RequestEnvelope → Backend Service → External Target → ResponseEnvelope
```

## Available Backend Types

| Service | Use Case |
|---------|----------|
| `http` | Generic HTTP/HTTPS passthrough |
| `fhir` | FHIR R4 resource servers |
| `dicomweb` | DICOMweb QIDO-RS/WADO-RS/STOW-RS |
| `http3` | HTTP/3 over QUIC |
| `dicom_scu` | DICOM SCU operations (C-FIND, C-MOVE, C-STORE) |
| `jmix` | JMIX packaging |
| `echo` | Echo/test backend |
| `mock_dicom` | Mock DICOM responses for testing |

## Configuration Patterns

### Using Targets (Recommended)

Define connection once, reuse across backends:

**config.toml:**
```toml
[targets.prod_api]
connection.host = "api.example.com"
connection.protocol = "https"
authentication.method = "bearer"
timeout_secs = 60
```

**pipelines/main.toml:**
```toml
[backends.my_backend]
service = "http"
target_ref = "prod_api"
```

### Direct Configuration

```toml
[backends.my_backend]
service = "http"
[backends.my_backend.options]
base_url = "https://api.example.com/v1"
```

## Common Backend Configs

### HTTP API
```toml
[backends.external_api]
service = "http"
[backends.external_api.options]
base_url = "https://api.example.com/v1"
```

### FHIR Server
```toml
[backends.fhir_server]
service = "fhir"
[backends.fhir_server.options]
base_url = "https://hapi.fhir.org/baseR4"
```

### DICOMweb PACS
```toml
[backends.pacs]
service = "dicomweb"
[backends.pacs.options]
base_url = "https://pacs.example.com/dicomweb"
```

### HTTP/3 Backend
```toml
[backends.h3_api]
service = "http3"
[backends.h3_api.options]
host = "fast-api.example.com"
port = 443
base_path = "/v2"
ca_cert_path = "/path/to/ca.pem"  # Optional for self-signed
```

### DICOM SCU
```toml
[backends.pacs_scu]
service = "dicom_scu"
[backends.pacs_scu.options]
host = "pacs.hospital.local"
port = 104
ae_title = "HARMONY"
called_ae_title = "PACS"
```

## Adding a New Backend Type

If you need a new backend type (not just configuration):

### Files to Modify

1. **Service implementation**: `src/services/<your_service>.rs`
2. **Service type enum**: `src/models/service.rs`
3. **Service registration**: `src/services/mod.rs`

### Implementation Pattern

```rust
pub struct YourService;

#[async_trait]
impl BackendService for YourService {
    async fn execute(
        &self,
        envelope: RequestEnvelope,
        options: &BackendOptions,
    ) -> Result<ResponseEnvelope, BackendError> {
        // Convert envelope to your protocol
        // Make external request
        // Convert response back to ResponseEnvelope
    }
}
```

## Pipeline Integration

Reference backend in pipeline:

```toml
[pipelines.my_pipeline]
networks = ["default"]
endpoints = ["my_endpoint"]
backends = ["my_backend"]
middleware = ["auth", "transform"]
```

## Testing

Test backend configuration by running Harmony with `--validate-config`:
```bash
./harmony --config config.toml --validate-config
```

Or run integration tests:
```bash
cargo test --test http_backend
```
