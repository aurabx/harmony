# DIMSE Integration

The harmony-proxy now integrates with a dedicated `dimse` crate that provides native DICOM Message Service Element (DIMSE) protocol support. This integration allows the proxy to handle both DICOM endpoint operations (SCP - Service Class Provider) and backend operations (SCU - Service Class User).

## Architecture

```
HTTP Request → Harmony Proxy → DIMSE Crate → DICOM Network
                    ↓
              Service Types:
              - Endpoint (SCP): HTTP→DIMSE bridge  
              - Backend (SCU): Outbound DICOM operations
```

## Service Configuration

The `dicom` service type supports two distinct usage patterns:

### Backend Usage (SCU - Service Class User)

When configured as a backend, the DICOM service acts as an SCU performing outbound DICOM operations:

```toml
[backends.my_pacs]
service = "dicom"

[backends.my_pacs.options]
aet = "ORTHANC"
host = "localhost"
port = 4242
local_aet = "HARMONY_SCU"
use_tls = false
```

**Supported Operations:**
- `C-ECHO`: Test connectivity to remote DICOM node
- `C-FIND`: Query remote DICOM node for studies/series/images
- `C-MOVE`: Request remote node to move datasets

### Endpoint Usage (SCP - Service Class Provider)

When configured as an endpoint, the DICOM service accepts DICOM network connections (SCP). Inbound DIMSE is converted to Harmony Request Envelopes and processed by the pipeline that references this endpoint.

```toml
[endpoints.dicom_scp]
service = "dicom"

[endpoints.dicom_scp.options]
local_aet = "HARMONY_SCP"
# bind_addr = "0.0.0.0"
# port = 11112
```

Important:
- The pipeline determines how inbound DICOM is processed. The pipeline references the DICOM endpoint in its endpoints list, and the SCP is started automatically for that pipeline.
- To build an HTTP→DICOM bridge, use an HTTP endpoint together with a DICOM backend (SCU) in the same pipeline (see below).

## Usage Examples

### Backend SCU Operations

HTTP requests to backends configured with DICOM service will trigger DIMSE operations.

Assuming an HTTP endpoint with appropriate transforms:

```bash
# Trigger C-ECHO via backend
curl -X POST http://localhost:8080/trigger-dicom/echo

# Trigger C-FIND via backend  
curl -X POST http://localhost:8080/trigger-dicom/find \
  -H "Content-Type: application/json" \
  -d '{"patient_id": "12345", "query_level": "PATIENT"}'
```

### HTTP→DICOM Bridge (via HTTP endpoint + DICOM backend)

Use an HTTP endpoint to expose routes, and a DICOM backend (SCU) to perform outbound DIMSE operations. Example:

```toml
[pipelines.dicom_backend_demo]
description = "Demo DICOM backend usage - HTTP request triggers DIMSE SCU operations"
networks = ["default"]
endpoints = ["http_to_dicom"]
middleware = []
backends = ["dicom_pacs"]

[endpoints.http_to_dicom]
service = "http"
options = { path_prefix = "/dicom" }

[backends.dicom_pacs]
service = "dicom"

[backends.dicom_pacs.options]
aet = "ORTHANC"
host = "localhost"
port = 4242
local_aet = "HARMONY_SCU"
```

Then invoke via HTTP:
```bash
# C-ECHO via backend
curl -X POST http://localhost:8080/dicom/echo

# C-FIND via backend  
curl -X POST http://localhost:8080/dicom/find \
  -H "Content-Type: application/json" \
  -d '{"patient_id": "12345", "query_level": "PATIENT"}'
```

## Implementation Status

### ✅ Completed
- **DIMSE Crate Foundation**: Separate crate with proper DICOM dependencies
- **Dual Service Support**: Single service type supports both backend and endpoint usage
- **Configuration Integration**: Seamlessly integrated with existing service architecture
- **SCU Operations (via DCMTK)**: C-ECHO and C-FIND wired through `echoscu`/`findscu`, exercised end-to-end in tests
- **C-FIND Dataset Extraction/Streaming**: Responses extracted (`-X`) and streamed back as datasets; artifacts preserved under `./tmp`
- **Validation**: Proper configuration validation for both usage patterns

### 🚧 Stub / Scaffold
- **C-MOVE**: Not implemented

### 📋 Planned Enhancements
1. **Native DIMSE Protocol**: Implement SCU/SCP with `dicom-ul` (replace DCMTK CLI usage)
2. **C-STORE and C-MOVE**: Add full support for store and move operations
3. **TLS Support**: Secure DICOM connections for SCU/SCP
4. **Hardening & Observability**: Robust error handling, metrics, and logs across DIMSE flows

## Configuration Examples

See `examples/default/pipelines/dimse-integration.toml` for a complete configuration demonstrating both backend (SCU) and endpoint (SCP) usage patterns.