# Backends

**Last Updated**: 2026-09-24

Backends enable the pipeline to communicate with external systems (targets). Backends operate within the unified `PipelineExecutor` and handle the "backend invocation" step of the pipeline.

> **Custom Backends**: To create your own backend service types, see [extensions.md](extensions.md#service-factories).

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

The backend inherits the target's `authentication`. The value must be the bare key. A reference that does not resolve stops the config from loading. See [Authentications](configuration.md#authentications).

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

### HTTP (Passthru)

A basic HTTP backend for connecting to HTTP/HTTPS targets.

**Service behavior**:
- Accepts a `RequestEnvelope` and converts it to an HTTP request
- If a `Command` is set on the envelope, the Command's `Operation` overrides the HTTP method (e.g., Query/Retrieve→GET, Store→POST, Update→PUT). See [commands.md](commands.md).
- Sends request to configured target
- Converts HTTP response back to `ResponseEnvelope`
- Forwards request headers as described in [Request Headers Sent Upstream](#request-headers-sent-upstream)
- Preserves response headers, status codes, and body

**Configuration**:
```toml
[backends.<name>]
service = "http"
[backends.<name>.options]
base_url = "https://api.example.com"
``` path=null start=null

**Example**: HTTP API backend
```toml
[backends.external_api]
service = "http"
[backends.external_api.options]
base_url = "https://external-api.example.com/v1"
``` path=null start=null

### FHIR

Extends the HTTP backend for FHIR resource servers.

**Service behavior**:
- FHIR-aware request/response handling
- Supports FHIR search parameters and operations
- Validates FHIR content types

**Configuration**:
```toml
[backends.<name>]
service = "fhir"
[backends.<name>.options]
base_url = "https://fhir.example.com/r4"
``` path=null start=null

**Example**: FHIR R4 server
```toml
[backends.fhir_server]
service = "fhir"
[backends.fhir_server.options]
base_url = "https://hapi.fhir.org/baseR4"
``` path=null start=null

### DICOMweb

Extends the HTTP backend for DICOMweb QIDO-RS/WADO-RS/STOW-RS targets.

**Service behavior**:
- DICOMweb-compliant request formatting
- Multipart handling for STOW-RS
- QIDO-RS query parameter construction

**Configuration**:
```toml
[backends.<name>]
service = "dicomweb"
[backends.<name>.options]
base_url = "https://dicomweb.example.com"
``` path=null start=null

**Example**: DICOMweb PACS
```toml
[backends.pacs_dicomweb]
service = "dicomweb"
[backends.pacs_dicomweb.options]
base_url = "https://pacs.example.com/dicomweb"
``` path=null start=null

### HTTP/3 (QUIC)

An HTTP/3 backend for connecting to targets over QUIC with TLS 1.3.

**Service behavior**:
- Connects to backends using HTTP/3 over QUIC (UDP transport)
- Built-in TLS 1.3 encryption (always enabled)
- Multiplexed streams without head-of-line blocking
- Supports custom CA certificates for self-signed servers
- If a `Command` is set on the envelope, the Command's `Operation` overrides the HTTP method, same as the HTTP backend. See [commands.md](commands.md).

**Configuration**:
```toml
[backends.<name>]
service = "http3"
[backends.<name>.options]
host = "api.example.com"
port = 443
base_path = "/api/v1"
ca_cert_path = "/path/to/ca.pem"  # Optional: for self-signed certs
timeout_secs = 30                  # Optional: request timeout
``` path=null start=null

Alternatively, use the connection config format:
```toml
[backends.<name>]
service = "http3"
[backends.<name>.options.connection]
host = "api.example.com"
port = 443
protocol = "h3"
base_path = "/api/v1"
ca_cert_path = "/path/to/ca.pem"
``` path=null start=null

**Configuration Options**:
- `host` (string, required): Target hostname
- `port` (integer, optional): Target port (default: 443)
- `base_path` (string, optional): Base URL path prefix
- `ca_cert_path` (string, optional): Path to PEM-encoded CA certificate for validating self-signed or custom CA certificates
- `timeout_secs` (integer, optional): Request timeout in seconds (default: 30)

**Example**: HTTP/3 API backend
```toml
[backends.h3_api]
service = "http3"
[backends.h3_api.options]
host = "fast-api.example.com"
port = 443
base_path = "/v2"
``` path=null start=null

**Example**: HTTP/3 with custom CA
```toml
[backends.internal_h3]
service = "http3"
[backends.internal_h3.options]
host = "internal.corp.local"
port = 8443
ca_cert_path = "./certs/internal-ca.pem"
``` path=null start=null

**When to use HTTP/3**:
- High-latency or unreliable networks (mobile, satellite)
- Backend servers that support HTTP/3
- When you need connection migration (e.g., mobile clients changing networks)
- To avoid TCP head-of-line blocking on multiplexed connections

### DICOM SCU (Service Class User)

A DICOM DIMSE backend for connecting to remote DICOM PACS via C-ECHO/C-FIND/C-MOVE/C-GET/C-STORE operations.

**Service behavior**:
- Converts `RequestEnvelope` to DICOM DIMSE operations (SCU - outgoing requests)
- Uses `resolve_operation()` to determine the DIMSE action: Query→C-FIND, Retrieve→C-GET/C-MOVE, Store→C-STORE, Ping→C-ECHO. Falls back to path-based detection if no Command is set. See [commands.md](commands.md).
- Communicates with remote DICOM nodes using AE titles
- Converts DICOM responses back to `ResponseEnvelope`
- Supports C-ECHO, C-FIND, C-MOVE, C-GET, C-STORE operations

**Configuration**:
```toml
[backends.<name>]
service = "dicom_scu"
[backends.<name>.options]
aet = "REMOTE_AET"           # Remote Application Entity Title
host = "pacs.example.com"
port = 4242
local_aet = "HARMONY_SCU"    # Local AE title
dimse_retrieve_mode = "get" # DICOM retrieval mode: "get" or "move"
use_tls = false
``` path=null start=null

**Configuration Options**:
- `aet` (string, required): Remote Application Entity Title
- `host` (string, required): PACS hostname or IP address
- `port` (integer, required): PACS port number
- `local_aet` (string, optional): Local AE title (default: "HARMONY_SCU")
- `dimse_retrieve_mode` (string, optional): DICOM retrieval mode (default: "get")
  - `"get"` (C-GET): Direct image retrieval, works without PACS-side AE configuration
  - `"move"` (C-MOVE): Requires PACS to know SCU's AE title and network address
- `use_tls` (boolean, optional): Enable TLS encryption (default: false)
- `incoming_store_port` (integer, optional): Port for C-STORE SCP when using C-MOVE
- `persistent_store_scp` (boolean, optional): Keep persistent C-STORE SCP listening

**Example**: DICOM PACS backend
```toml
[backends.orthanc_pacs]
service = "dicom_scu"  # Use "dicom_scu" for SCU backend
[backends.orthanc_pacs.options]
aet = "ORTHANC"
host = "localhost"
port = 4242
local_aet = "HARMONY_SCU"
dimse_retrieve_mode = "get"
incoming_store_port = 11112
persistent_store_scp = true
use_tls = false
``` path=null start=null

**Prerequisites**: DCMTK is needed for C-MOVE only (see [dimse-integration.md](dimse-integration.md))

**Supported Operations**:
- `C-ECHO`: Test connectivity
- `C-FIND`: Query for studies/series/images
- `C-MOVE`: Request dataset transfer
- `C-GET`: Retrieve datasets
- `C-STORE`: Send one instance to the PACS per request

**C-STORE request body**:

| Body | Sent by | Result |
|------|---------|--------|
| A DICOM Part 10 file | An HTTP client | The instance is sent |
| A JSON object with the Part 10 file as base64 in `part10_b64` | The `dicom_scp` endpoint in the same pipeline | The instance is sent |
| A JSON object without `part10_b64` | | Rejected |
| Any other body that does not start with `{` | An HTTP client | Sent as bare data set bytes, read as Implicit VR Little Endian |
| A body that starts with `{` and is not valid JSON | | Rejected |

The backend never opens a file that a request names. A JSON body with a `file`
key and no `part10_b64` is rejected.

The backend sends the instance in `part10_b64` as it was received. A change
that middleware makes to the `dataset` key does not change what is sent. A
`transform` that removes patient tags from `dataset` does not de-identify the
instance that reaches the PACS.

A middleware that replaces the whole body, such as `transform`, must keep
`part10_b64` in its output. Without it the backend has no instance to send and
the store fails.

The backend reads the transfer syntax of the instance from the file meta group
and proposes it to the PACS:

| Instance | Transfer syntaxes proposed |
|----------|----------------------------|
| Compressed pixel data, for example JPEG Lossless | The transfer syntax of the instance |
| Uncompressed pixel data | The transfer syntax of the instance, then Implicit VR Little Endian |
| Deflated Explicit VR Little Endian | Explicit VR Little Endian, then Implicit VR Little Endian. The instance is sent without deflate compression |
| Bare data set bytes | Implicit VR Little Endian |

The backend does not transcode pixel data. A compressed instance reaches the
PACS in the transfer syntax it was received in. If the PACS does not accept
that transfer syntax, the store fails and nothing is sent. The error names the
SOP class and the transfer syntax.

The backend checks the pixel data against the negotiated transfer syntax
before it sends anything. It refuses an instance whose label and pixel data
disagree: compressed fragments under an uncompressed transfer syntax, or
uncompressed pixel values under a compressed one. The error names the
negotiated transfer syntax and the form of the pixel data. An instance with no
pixel data, such as a structured report, is sent in any transfer syntax.

A `dicom_scp` endpoint in the same pipeline records the transfer syntax it
negotiated with the sending device, so the instance keeps it on the way to the
PACS.

A DICOMweb STOW-RS request reaches this backend as one C-STORE per part. The
`dicomweb` endpoint takes the parts out of the multipart body.

The response is JSON. Read `success` to learn the result:

```json
{"operation":"store","success":true,"remote_aet":"ORTHANC","host":"localhost","port":4242,"sop_class_uid":"1.2.840.10008.5.1.4.1.1.2","sop_instance_uid":"1.2.3"}
```

A failed store has `"success": false` and an `error` string. A `dicom_scp`
endpoint in front of this backend then answers the sending device with status
`0xC000`.

See [dimse-integration.md](dimse-integration.md) for detailed DIMSE usage.

### Storage (Filesystem/S3)

A backend for reading and writing files to local storage or S3-compatible object storage.

**Service behavior**:
- Uses `resolve_operation()` to determine read vs. write: Query/Retrieve→read, Store/Update/Patch→write. Falls back to HTTP method if no Command is set. See [commands.md](commands.md).
- **Read** (GET or Query/Retrieve): Reads file at path resolved from `read_pattern`
- **Write** (POST/PUT or Store/Update): Writes request body to path resolved from `write_pattern`
- Supports path templating with metadata, UUIDs, and timestamps
- Returns `Location` header and JSON status on successful write

**Configuration**:
```toml
[backends.<name>]
service = "storage"
[backends.<name>.options]
root = "./data"  # Local path or "s3://bucket/prefix"
read_pattern = "files/{uuid}.bin"
write_pattern = "files/{uuid}.bin"
``` path=null start=null

**S3 Configuration**:
To use S3, set `root` to an S3 URI (`s3://bucket-name/prefix`) and provide credentials:
```toml
[backends.s3_storage]
service = "storage"
[backends.s3_storage.options]
root = "s3://my-bucket/uploads"
region = "us-east-1"
access_key_id = "..."      # Optional (can use env vars)
secret_access_key = "..."  # Optional (can use env vars)
endpoint = "..."           # Optional (for MinIO/custom S3)
``` path=null start=null

**Path Templating**:
Patterns can use placeholders replaced at runtime:
- `{uuid}`: Generates a random UUID (v4)
- `{timestamp}`: Current timestamp (RFC3339)
- `{metadata_key}`: Replaced by value from request metadata (e.g. `{tenant}` from JWT)
- `{field_name}`: Replaced by value from normalized data (e.g. `{PatientID}`)

**Example**:
```toml
[backends.patient_docs]
service = "storage"
[backends.patient_docs.options]
root = "./patient_data"
# Write to: ./patient_data/{tenant}/{PatientID}/{uuid}.dcm
write_pattern = "{tenant}/{PatientID}/{uuid}.dcm"
``` path=null start=null

### Echo (Test)

A simple echo backend that reflects the request back as the response.

**Service behavior**:
- Returns the request envelope as the response
- Useful for testing and debugging pipelines
- No external communication

**Configuration**:
```toml
[backends.<name>]
service = "echo"
``` path=null start=null

**Example**: Echo test backend
```toml
[backends.test_echo]
service = "echo"
``` path=null start=null

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
- Uses `resolve_operation()` to determine the operation: Query→C-FIND, Retrieve→C-GET, Ping→C-ECHO. Falls back to path-based detection if no Command is set. See [commands.md](commands.md).
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

- [commands.md](commands.md) - Command system for protocol-agnostic operations
- [router.md](router.md) - Pipeline execution flow
- [endpoints.md](endpoints.md) - Protocol entry points
- [adapters.md](adapters.md) - Protocol adapter architecture
- [dimse-integration.md](dimse-integration.md) - DICOM DIMSE details

