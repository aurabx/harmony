# Backends

**Last Updated**: 2025-01-18 (Phase 6)

Backends enable the pipeline to communicate with external systems (targets). Backends operate within the unified `PipelineExecutor` and handle the "backend invocation" step of the pipeline.

## Architecture

```
RequestEnvelope → Backend Service → External Target → ResponseEnvelope
```

**Backend responsibilities**:
- Convert `RequestEnvelope` to protocol-specific requests
- Communicate with external targets (HTTP endpoints, DICOM PACS, databases, etc.)
- Convert responses back to `ResponseEnvelope`
- Handle target selection when multiple targets are configured

**Note**: Backends run inside the `PipelineExecutor` (see [router.md](router.md)), not in protocol adapters. All protocols use the same backend implementations.

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
```

**Example**: HTTP API backend
```toml
[backends.external_api]
service = "http"
[backends.external_api.options]
base_url = "https://external-api.example.com/v1"
```

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
```

**Example**: FHIR R4 server
```toml
[backends.fhir_server]
service = "fhir"
[backends.fhir_server.options]
base_url = "https://hapi.fhir.org/baseR4"
```

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
```

**Example**: DICOMweb PACS
```toml
[backends.pacs_dicomweb]
service = "dicomweb"
[backends.pacs_dicomweb.options]
base_url = "https://pacs.example.com/dicomweb"
```

### DICOM (DIMSE)

A DICOM DIMSE backend for connecting to DICOM PACS via C-ECHO/C-FIND/C-MOVE/C-GET operations.

**Service behavior**:
- Converts `RequestEnvelope` to DICOM DIMSE operations (SCU)
- Communicates with DICOM nodes using AE titles
- Converts DICOM responses back to `ResponseEnvelope`
- Supports C-ECHO, C-FIND, C-MOVE, C-GET operations

**Configuration**:
```toml
[backends.<name>]
service = "dicom"
[backends.<name>.options]
aet = "REMOTE_AET"           # Remote Application Entity Title
host = "pacs.example.com"
port = 4242
local_aet = "HARMONY_SCU"    # Local AE title
dimse_retrieve_mode = "get" # DICOM retrieval mode: "get" or "move"
use_tls = false
```

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
service = "dicom"
[backends.orthanc_pacs.options]
aet = "ORTHANC"
host = "localhost"
port = 4242
local_aet = "HARMONY_SCU"
dimse_retrieve_mode = "get"
incoming_store_port = 11112
persistent_store_scp = true
use_tls = false
```

**Prerequisites**: Requires DCMTK installed (see [dimse-integration.md](dimse-integration.md))

**Supported Operations**:
- `C-ECHO`: Test connectivity
- `C-FIND`: Query for studies/series/images
- `C-MOVE`: Request dataset transfer
- `C-GET`: Retrieve datasets

See [dimse-integration.md](dimse-integration.md) for detailed DIMSE usage.

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
```

**Example**: Echo test backend
```toml
[backends.test_echo]
service = "echo"
```

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

## See Also

- [router.md](router.md) - Pipeline execution flow
- [endpoints.md](endpoints.md) - Protocol entry points
- [adapters.md](adapters.md) - Protocol adapter architecture
- [dimse-integration.md](dimse-integration.md) - DICOM DIMSE details

