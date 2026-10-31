---
title: Backends
description: Backend types, connection settings, upstream headers, and target selection.
status: active
updated: 2026-10-31
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
- Any client header named in the client's `Connection` header. A header Harmony set stays, so a client cannot remove the mesh JWT.
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

If a backend has `bearer` or `basic` authentication and middleware also sets `Authorization`, the backend's authentication wins and the middleware header is not sent. Such a mesh egress backend sends its own credential, not the mesh JWT. With `api_key` authentication under another header name, the upstream receives both the key and the mesh JWT.

`forward_authorization` must be a boolean. Any other value fails the request with an error that names the option. Dropping a client `Authorization` header is logged at `debug` level. The header value is never logged.

**Client copy of the target's credential header**: replaced. The backend's authentication replaces any header with the same name, in any letter case, and treats `_` as `-`. A client that sends its own `X-API-Key` to a backend whose `api_key` authentication uses that `header_name` does not reach the upstream with it, and the upstream receives one value. `http_streaming_backend` sends the backend's authentication in the same way as the other backends.

**Everything else** passes through unchanged. This includes `Accept`, `Content-Type` and custom `X-` headers.

## Request Paths and Redirects

A request path with a `.` or `..` segment gets 400 and never reaches middleware or a backend. This includes percent-encoded forms such as `%2e%2e`. It applies to HTTP and HTTP/3, on every endpoint and mesh ingress.

The `http`, `http3`, `fhir`, `jmix_backend` and `http_streaming_backend` backends apply the same check to the path they send, because middleware can change that path after the first check. A `transform` that writes such a value to `target_details.uri` fails the request before anything reaches the upstream. See [Transform Middleware](middleware-transforms.md#transform-jolt).

A backend with `authentication` follows a redirect only to the same origin (scheme, host and port), for up to 10 redirects. A redirect to another origin is not followed, so the credential stays with the configured upstream. The `http`, `fhir` and `jmix_backend` backends return the 3xx response to the client, and `http_streaming_backend` fails the request. A backend with no `authentication` follows redirects to any origin. No backend follows a redirect from https to http. The `webhook` middleware uses the same rules. The `dimse_to_jmix` upload does not follow a redirect from https to http either, and the upload then fails.

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

