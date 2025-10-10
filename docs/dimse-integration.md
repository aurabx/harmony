# DIMSE Integration

Harmony integrates DIMSE via a dedicated `dimse` crate that orchestrates DCMTK CLI tools for networking today (SCU/SCP). Native DIMSE networking is planned. This enables both DICOM endpoint operations (SCP - Service Class Provider) and backend operations (SCU - Service Class User).

## Prerequisites

- DCMTK must be installed and available on PATH when using DICOM DIMSE features
  - Tools used: `echoscu`, `findscu`, `movescu`, `getscu`, and a persistent `storescp`
  - macOS (Homebrew): `brew install dcmtk`
  - Debian/Ubuntu: `sudo apt-get install dcmtk`

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
- **DIMSE Orchestration via DCMTK**: SCU operations (C-ECHO, C-FIND, C-GET, C-MOVE) use `echoscu`/`findscu`/`getscu`/`movescu`
- **Persistent Store SCP**: By default Harmony launches a persistent `storescp` for C-STORE delivery when using a DICOM backend in persistent mode
- **Dual Service Support**: Single service type supports both backend and endpoint usage
- **Configuration Integration**: Seamlessly integrated with existing service architecture
- **C-FIND Dataset Extraction/Streaming**: Responses extracted (`-X`) and streamed back as datasets; artifacts preserved under `./tmp`
- **C-GET/C-MOVE Streaming**: All files written by DCMTK receivers in the operation output directory are streamed back (DCMTK may produce files without `.dcm` extensions, e.g. `SC.<SOPInstanceUID>`) 
- **Validation**: Proper configuration validation for both usage patterns

### 🚧 Stub / Scaffold
- Native DIMSE (non-DCMTK) networking (planned)

### 📋 Planned Enhancements
1. **Native DIMSE Protocol**: Implement SCU/SCP with `dicom-ul` (replace DCMTK CLI usage)
2. **TLS Support**: Secure DICOM connections for SCU/SCP
3. **Hardening & Observability**: Robust error handling, metrics, and logs across DIMSE flows

## Configuration Examples

See `examples/default/pipelines/dimse-integration.toml` for a complete configuration demonstrating both backend (SCU) and endpoint (SCP) usage patterns.

## Notes and Tips

- DCMTK requirement: Harmony relies on the DCMTK CLI for DIMSE networking. Ensure DCMTK is installed and on PATH before using DICOM endpoints/backends.
- DCMTK CLI tag format: when passing keys via `-k`, use plain tag form `gggg,eeee` without parentheses. For example, set the QueryRetrieveLevel and keys like:
  - `-k 0008,0052=STUDY`
  - `-k 0020,000D=<StudyInstanceUID>`
  - `-k 0010,0020=<PatientID>`
- C-MOVE destination listener: `movescu` must listen for incoming C-STORE. Harmony config exposes `incoming_store_port`; the SCU uses DCMTK’s `+P <port>` option and `-aem <DEST_AET>`. Ensure the QR SCP’s HostTable includes the destination AET with the same host/port.
- Test artifacts under `./tmp`:
  - C-FIND: `./tmp/dcmtk_find_<uuid>/rsp*.dcm`
  - C-GET:  `./tmp/dcmtk_get_<uuid>/*`
  - C-MOVE: `./tmp/dcmtk_move_<uuid>/*`
  - Last MOVE debug payload: `./tmp/movescu_last.json`
- Debugging: set `HARMONY_TEST_DEBUG=1` to attach the last `movescu` arguments/stdout/stderr to HTTP responses (where applicable).
- Test verbosity: DCMTK child process output is suppressed in tests by default. Set `HARMONY_TEST_VERBOSE_DCMTK=1` to enable verbose DCMTK logs (adds `-d` to `dcmqrscp` and shows child stdout/stderr).
- Test data: if `dev/samples` exists, tests may preload a limited number of `.dcm` files into the QR SCP via `storescu` prior to MOVE operations.

## Troubleshooting

- Association rejected with BadAppContextName in QR SCP logs
  - This can appear if a readiness probe uses a raw TCP connect during startup (no DICOM PDU). It is safe to ignore if a subsequent valid association is accepted.

- No instances received after C-MOVE
  - Verify that the destination AET in the MOVE request matches a HostTable entry on the QR SCP with the correct host and port. For example:
    - `HARMONY_MOVE = (HARMONY_MOVE, 127.0.0.1, 11124)`
  - Ensure the SCU is listening for incoming C-STORE on that same port. DCMTK movescu uses `+P <port>` (some older docs mention `-pm`, but Homebrew DCMTK 3.6.9 uses `+P`).
  - Use Study Root for MOVE (`-S`) when matching by `StudyInstanceUID`.
  - Check movescu debug at `./tmp/movescu_last.json` for args, stdout, stderr, and status.

- C-MOVE/C-GET produce no files even though DCMTK reports success
  - DCMTK may write files without a `.dcm` extension (e.g., `SC.<SOPInstanceUID>`). Tools or code expecting only `.dcm` names might miss them.
  - In Harmony, we stream back all files in the DCMTK output directory, regardless of extension.

- DCMTK `-k` key errors like "bad key format"
  - Use `gggg,eeee` format without parentheses. Examples: `0008,0052=STUDY`, `0020,000D=<UID>`, `0010,0020=<PatientID>`.

- Duplicate or quota warnings in dcmqrscp logs
  - dcmqrscp may delete stored files due to duplicate SOP Instance UID or internal quotas while still indexing the dataset. This can be normal for tests and not an error.

- Debugging tips
  - Run pre-MOVE diagnostics with `findscu` at STUDY level and `-X -od` to see matched identifiers.
  - Set `HARMONY_TEST_DEBUG=1` to include `movescu_last.json` in API responses for MOVE.
