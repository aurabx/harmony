# Backends

**Last Updated**: 2025-01-31

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
[targets.prod_api]
connection.host = "api.example.com"
connection.protocol = "https"
authentication.method = "bearer"
timeout_secs = 60
``` path=null start=null

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

## Backend Types

### HTTP (Passthru)

A basic HTTP backend for connecting to HTTP/HTTPS targets.

**Service behavior**:
- Accepts a `RequestEnvelope` and converts it to an HTTP request
- Sends request to configured target
- Converts HTTP response back to `ResponseEnvelope`
- Preserves headers, status codes, and body

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

A DICOM DIMSE backend for connecting to remote DICOM PACS via C-ECHO/C-FIND/C-MOVE/C-GET operations.

**Service behavior**:
- Converts `RequestEnvelope` to DICOM DIMSE operations (SCU - outgoing requests)
- Communicates with remote DICOM nodes using AE titles
- Converts DICOM responses back to `ResponseEnvelope`
- Supports C-ECHO, C-FIND, C-MOVE, C-GET operations

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

**Prerequisites**: Requires DCMTK installed (see [dimse-integration.md](dimse-integration.md))

**Supported Operations**:
- `C-ECHO`: Test connectivity
- `C-FIND`: Query for studies/series/images
- `C-MOVE`: Request dataset transfer
- `C-GET`: Retrieve datasets

See [dimse-integration.md](dimse-integration.md) for detailed DIMSE usage.

### Storage (Filesystem/S3)

A backend for reading and writing files to local storage or S3-compatible object storage.

**Service behavior**:
- **GET**: Reads file at path resolved from `read_pattern`
- **POST/PUT**: Writes request body to path resolved from `write_pattern`
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

- [router.md](router.md) - Pipeline execution flow
- [endpoints.md](endpoints.md) - Protocol entry points
- [adapters.md](adapters.md) - Protocol adapter architecture
- [dimse-integration.md](dimse-integration.md) - DICOM DIMSE details

