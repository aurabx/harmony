---
title: Backends
description: Backend types, connection settings, upstream headers, and target selection.
status: active
updated: 2026-10-24
type: Configuration
---

# Backends

Backends enable the pipeline to communicate with external systems (targets). Backends operate within the unified `PipelineExecutor` and handle the "backend invocation" step of the pipeline.

> **Custom Backends**: To create your own backend service types, see [Extension Types](../architecture/extensions-types.md#service-factories).

## Architecture

``` path=null start=null
RequestEnvelope → Backend Service → External Target → ResponseEnvelope
``` path=null start=null

**Backend responsibilities**:
- Convert `RequestEnvelope` to protocol-specific requests
- Communicate with external targets (HTTP endpoints, DICOM PACS, databases, etc.)
- Convert responses back to `ResponseEnvelope`
- Handle target selection when multiple targets are configured

## Connection Configuration

Backends can define connection details directly or reference a shared **Target**.

### Using Targets (Recommended)

Define a target in your main configuration and reference it from the backend. This allows connection reuse and centralized management.

**config.toml**:
```toml
[authentications.prod_api_token]
id = "prod_api_token"
method = "bearer"
[authentications.prod_api_token.options]
token = "$PROD_API_TOKEN"

[targets.prod_api]
connection.host = "api.example.com"
connection.protocol = "https"
authentication = "prod_api_token"  # Bare key from [authentications]
timeout_secs = 60
``` path=null start=null

The backend inherits the target's `authentication`. The value must be the bare key. A reference that does not resolve stops the config from loading. See [Authentications](configuration-file.md#authentications).

**pipelines/main.toml**:
```toml
[backends.my_backend]
service = "http"
target_ref = "prod_api"  # Inherits connection settings from 'prod_api'
``` path=null start=null

### Direct Configuration

You can also define connection settings directly on the backend:

```toml
[backends.my_backend]
service = "http"
[backends.my_backend.connection]
host = "api.example.com"
protocol = "https"
``` path=null start=null

### Legacy Configuration (Options)

Service-specific options are still supported for backward compatibility:

```toml
[backends.my_backend]
service = "http"
[backends.my_backend.options]
base_url = "https://api.example.com"
``` path=null start=null

## Request Headers Sent Upstream

HTTP-based backends (`http`, `http3`, `fhir`, `dicomweb`, `jmix_backend`, `http_streaming_backend`) build the upstream request from the client's headers plus any headers middleware added. Every backend applies the same filter before sending.

**Always removed**:
- `Host`. The HTTP client sets it from the backend URL.
- Hop-by-hop headers (RFC 9110 §7.6.1): `Connection`, `Keep-Alive`, `Proxy-Authenticate`, `Proxy-Authorization`, `Proxy-Connection`, `TE`, `Trailer`, `Transfer-Encoding`, `Upgrade`.
- Any header named in the client's `Connection` header.
- `Content-Length`. The HTTP client sets it from the body it sends.

**Client `Authorization`**: removed by default. The client's credential is for Harmony (for example `basic_auth`, `jwt_auth` or a mesh JWT on ingress), not for the upstream.

To pass the client's `Authorization` through, set `forward_authorization = true` in the backend options:

```toml
[backends.passthrough_api]
service = "http"
[backends.passthrough_api.options]
base_url = "https://api.example.com"
forward_authorization = true   # default: false
``` path=null start=null

Harmony never sends two `Authorization` headers. When Harmony sends its own credential, the client's is dropped even with `forward_authorization = true`. Harmony's own credential is either:
- the backend's authentication (`authentication`, or the `authentication` of its `target_ref`), or
- an `Authorization` header set by middleware, such as the mesh egress JWT.

`forward_authorization` must be a boolean. Any other value fails the request with an error that names the option. Dropping a client `Authorization` header is logged at `debug` level. The header value is never logged.

**Everything else** passes through unchanged. This includes `Accept`, `Content-Type` and custom `X-` headers.

## Backend Types

### HTTP Backends

Configuration for the HTTP, FHIR, DICOMweb, and HTTP/3 backends. See [HTTP Backends](backends-http.md).

### DICOM, Storage, and Echo Backends

Configuration for the DICOM SCU, storage, and echo backends. See [DICOM, Storage, and Echo Backends](backends-dicom-storage.md).

## Target Selection

Backends can have multiple targets configured. The backend service decides which target to use based on:
- Load balancing strategy
- Health checks
- Request routing rules

**Example with multiple targets**:
```toml
[backends.load_balanced_api]
service = "http"
targets = ["api1", "api2"]

[targets.api1]
url = "https://api1.example.com"

[targets.api2]
url = "https://api2.example.com"
```

### Mock DICOM

A built-in mock backend that simulates a DICOM PACS. Useful for testing pipelines without requiring a real external PACS system.

**Service behavior**:
- Acts as a DICOM SCU/Target for pipeline backends
- Uses `resolve_operation()` to determine the operation: Query→C-FIND, Retrieve→C-GET, Ping→C-ECHO. Falls back to path-based detection if no Command is set. See [commands.md](../architecture/commands.md).
- Supports `C-FIND` (Patient, Study, Series, Image levels)
- Supports `C-GET` (Instance and Frame retrieval)
- Supports `C-ECHO`
- Provides built-in sample data (CT study with multiple series)
- Validates query parameters against mock data

**Configuration**:
```toml
[backends.mock_pacs]
service = "mock_dicom"
# No options required
```

**Supported Operations**:
- **Query (C-FIND)**:
    - Filters supported: `PatientID`, `StudyInstanceUID`, `SeriesInstanceUID`, `SOPInstanceUID`, `Modality`, `InstanceNumber`
    - Returns hierarchical results matching the query level
- **Retrieve (C-GET)**:
    - Returns mock DICOM file content
    - Supports frame retrieval (e.g., `.../frames/1`)
- **Echo (C-ECHO)**:
    - Always returns success

## See Also

- [commands.md](../architecture/commands.md) - Command system for protocol-agnostic operations
- [router.md](../architecture/router.md) - Pipeline execution flow
- [endpoints.md](endpoints.md) - Protocol entry points
- [adapters.md](../architecture/adapters.md) - Protocol adapter architecture
- [DIMSE Integration](../integrations/dicom-dimse.md) - DICOM DIMSE details

