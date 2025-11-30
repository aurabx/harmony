# Backends

**Last Updated**: 2025-11-30

Backends enable the pipeline to communicate with external systems (targets). Backends operate within the unified `PipelineExecutor` and handle the "backend invocation" step of the pipeline.

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
``` path=null start=null

## See Also

- [router.md](router.md) - Pipeline execution flow
- [endpoints.md](endpoints.md) - Protocol entry points
- [adapters.md](adapters.md) - Protocol adapter architecture
- [dimse-integration.md](dimse-integration.md) - DICOM DIMSE details

