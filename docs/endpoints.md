# Endpoints

**Last Updated**: 2025-11-30

Endpoints define protocol entry points into the Harmony pipeline. Each endpoint is associated with a **Service** that handles protocol-specific data conversion to/from the internal `Envelope` format.

## Architecture

Endpoints work with **Protocol Adapters** (see [adapters.md](adapters.md)) to process requests:

```
Protocol Request → Protocol Adapter → Service (via Endpoint config) → RequestEnvelope → Pipeline
```

The endpoint configuration specifies:
- Which service type to use (e.g., `http`, `fhir`, `jmix`, `dicomweb`, `dicom`)
- Service-specific options (e.g., `path_prefix` for HTTP endpoints, `local_aet` for DICOM)
- How the service should construct request/response envelopes

## Connection Configuration

Endpoints can define connection details directly or reference a shared **Peer**.

### Using Peers (Recommended)

Define a peer in your main configuration and reference it from the endpoint.

**config.toml**:
```toml
[peers.hospital_a]
connection.host = "hospital-a.example.com"
connection.protocol = "dicom"
authentication.method = "mutual_tls"
```

**pipelines/main.toml**:
```toml
[endpoints.dicom_listener]
service = "dicom_scp"
peer_ref = "hospital_a"  # Inherits connection settings
```

### Direct Configuration

```toml
[endpoints.dicom_listener]
service = "dicom_scp"
[endpoints.dicom_listener.connection]
host = "0.0.0.0"
port = 11112
protocol = "dicom"
```

**Supported protocols**: `http`, `https`, `h3` (HTTP/3), `dicom` (DIMSE SCP), `fhir`, `jmix`.

**Experimental/Planned protocols**: `hl7v2`, `custom` (dynamic loading not yet implemented)

## Endpoint Types

### HTTP (Passthru)

A basic HTTP endpoint handled by `HttpAdapter`.

**Service behavior**:
- Accepts an HTTP request and converts it into a `RequestEnvelope`
- Takes a `ResponseEnvelope` and converts it into an HTTP response
- Preserves headers, query parameters, and body

**Configuration**:
```toml
[endpoints.<name>]
service = "http"
[endpoints.<name>.options]
path_prefix = "/<some-path>"
```

**Example**: HTTP passthrough to backend
```toml
[endpoints.api_passthrough]
service = "http"
[endpoints.api_passthrough.options]
path_prefix = "/api"
```

### FHIR

Extends the HTTP endpoint with FHIR-specific handling.

**Service behavior**:
- Accepts an HTTP request and converts it into a `RequestEnvelope`
- Provides FHIR-aware request/response handling
- Takes a `ResponseEnvelope` and converts it into a FHIR-compliant HTTP response

**Configuration**:
```toml
[endpoints.<name>]
service = "fhir"
[endpoints.<name>.options]
path_prefix = "/<some-path>"
```

**Example**: FHIR resource server
```toml
[endpoints.fhir_server]
service = "fhir"
[endpoints.fhir_server.options]
path_prefix = "/fhir"
```

### JMIX

JMIX endpoint registers a strict, fixed set of routes for the JMIX healthcare data exchange format.

**Service behavior**:
- Registers fixed routes under the configured `path_prefix`
- Handles JMIX data package operations

**Supported routes** (under configured `path_prefix`):
- `GET {prefix}/api/jmix/{id}` - Retrieve JMIX package
- `GET {prefix}/api/jmix/{id}/manifest` - Retrieve package manifest
- `GET {prefix}/api/jmix?studyInstanceUid=...` - Query by Study Instance UID
- `POST {prefix}/api/jmix` - Create JMIX package

**Configuration**:
```toml
[endpoints.<name>]
service = "jmix"
[endpoints.<name>.options]
path_prefix = "/<some-path>"
```

**Example**: JMIX data exchange
```toml
[endpoints.jmix_exchange]
service = "jmix"
[endpoints.jmix_exchange.options]
path_prefix = "/data"
```

For development testing guidance, refer to the project's development documentation.

### DICOMweb

Provides DICOMweb QIDO-RS (Query) and WADO-RS (Retrieve) endpoints.

**Service behavior**:
- Accepts DICOMweb requests and converts them into `RequestEnvelope`
- Supports QIDO-RS queries and WADO-RS retrieval operations
- Currently returns 501 Not Implemented for all endpoints (skeleton implementation)

**Configuration**:
```toml
[endpoints.<name>]
service = "dicomweb"
[endpoints.<name>.options]
path_prefix = "/dicomweb"
```

**Supported routes**:
- `GET /dicomweb/studies` - Query for studies (QIDO-RS)
- `GET /dicomweb/studies/{study_uid}/series` - Query for series (QIDO-RS)
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances` - Query for instances (QIDO-RS)
- `GET /dicomweb/studies/{study_uid}/metadata` - Retrieve study metadata (WADO-RS)
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/metadata` - Retrieve series metadata (WADO-RS)
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/metadata` - Retrieve instance metadata (WADO-RS)
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}` - Retrieve instance (WADO-RS)
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/frames/{frame_numbers}` - Retrieve frames (WADO-RS)
- `GET /dicomweb/bulkdata/{bulk_data_uri}` - Bulk data retrieval (WADO-RS)

**Example**: DICOMweb PACS interface
```toml
[endpoints.dicomweb_pacs]
service = "dicomweb"
[endpoints.dicomweb_pacs.options]
path_prefix = "/pacs"
```

### DICOM SCP (Service Class Provider)

Provides DICOM DIMSE SCP endpoint for receiving incoming DICOM requests.

**Service behavior**:
- Accepts incoming DIMSE connections (C-FIND, C-MOVE, C-GET, C-ECHO, C-STORE)
- Converts DIMSE operations to `RequestEnvelope` via protocol adapter
- Routes requests through the pipeline system
- Returns DIMSE responses to the calling SCU

**Configuration**:
```toml
[endpoints.<name>]
service = "dicom_scp"
[endpoints.<name>.options]
local_aet = "HARMONY_SCP"    # Local Application Entity Title (required)
bind_addr = "0.0.0.0"        # Bind address (default: 0.0.0.0)
port = 11112                  # Listen port (default: 11112)
enable_echo = true            # Enable C-ECHO (default: true)
enable_find = true            # Enable C-FIND (default: false)
enable_move = true            # Enable C-MOVE (default: false)
enable_get = true             # Enable C-GET (default: false)
enable_store = true           # Enable C-STORE (default: false)
storage_dir = "./tmp/dimse"  # Storage directory (optional)
```

**Supported Operations**:
- `C-ECHO` - Connectivity test (enabled by default)
- `C-FIND` - Query for studies/series/images
- `C-MOVE` - Request dataset transfer to destination AET
- `C-GET` - Direct dataset retrieval
- `C-STORE` - Store incoming datasets

**Example**: DICOM Query/Retrieve SCP
```toml
[endpoints.dicom_qr_scp]
service = "dicom_scp"
[endpoints.dicom_qr_scp.options]
local_aet = "QR_SCP"
bind_addr = "0.0.0.0"
port = 11112
enable_echo = true
enable_find = true
enable_move = true
enable_get = true
storage_dir = "./data/dicom"
```

**Note**: The DICOM SCP endpoint uses the `DimseAdapter` protocol adapter, not HTTP. It listens on the configured port for incoming DIMSE associations.

### C-STORE Handling

When `enable_store` is set to `true`, Harmony accepts incoming DICOM objects (C-STORE requests).

1. **Reception**: The file is received and written to the configured storage backend (e.g., `./tmp/dimse/<uuid>.dcm`).
2. **Pipeline Execution**: Harmony executes the pipeline with a request payload containing metadata about the stored file.
3. **Response**: Once the pipeline executes successfully, a C-STORE Success status (0x0000) is returned to the caller.

**Pipeline Request Payload (JSON):**
```json
{
  "operation": "store",
  "dir": "/path/to/storage/dimse",
  "file": "/path/to/storage/dimse/unique-id.dcm"
}
```

This allows you to use middleware to process the file (e.g., extract metadata, forward to another system, or trigger a notification) after it has been safely stored.
