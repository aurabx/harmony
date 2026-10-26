---
title: DIMSE Integration
description: How to use DICOM DIMSE SCU and SCP services, including C-ECHO, C-FIND, C-MOVE, and C-GET.
status: active
updated: 2026-10-26
type: Integration
---

# DIMSE Integration

Harmony integrates DIMSE via:
- **DimseAdapter**: Native DICOM DIMSE network operations (SCP) using `dicom-ul` Rust library
- **DICOM Backend**: Native Service Class User (SCU) for outbound DIMSE operations using `dicom-ul`

## Prerequisites

- **Native Rust SCP and SCU**: the SCP listener and the C-ECHO, C-FIND, C-STORE and C-GET SCU operations are implemented on `dicom-ul`, with no external process
- **DCMTK is required** for C-MOVE on the SCU side (`movescu`) and for the integration tests
- **DCMTK tools**: Install for C-MOVE and integration testing
  - macOS (Homebrew): `brew install dcmtk`
  - Debian/Ubuntu: `sudo apt-get install dcmtk`

## Architecture (Phase 6)

```
DIMSE Network Request
  ↓
DimseAdapter (SCP listener)
  ↓ Converts to
RequestEnvelope
  ↓
PipelineExecutor
  ├─ Incoming Middleware
  ├─ Backend (SCU or HTTP)
  └─ Outgoing Middleware
  ↓
ResponseEnvelope
  ↓ Converts back
DimseAdapter
  ↓
DIMSE Network Response
```

**OR** HTTP → DICOM Backend:

```
HTTP Request
  ↓
HttpAdapter
  ↓ Converts to
RequestEnvelope
  ↓
PipelineExecutor
  ├─ Incoming Middleware (sets Command)
  └─ DICOM Backend (SCU) reads Command via resolve_operation()
  ↓
C-ECHO/C-FIND/C-MOVE/C-GET to PACS
```

Protocol-aware endpoints and middleware set the `Command` on the envelope. The DICOM backend reads the Command via `resolve_operation()` to select the DIMSE operation, with fallback to path-based detection when no command is set. See [commands.md](../architecture/commands.md).

See [adapters.md](../architecture/adapters.md) for protocol adapter details.

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
- `C-GET`: Retrieve datasets from the remote node
- `C-STORE`: Send one instance to the remote node per request

`dicom` is the legacy name of the `dicom_scu` service. See
[DICOM, Storage, and Echo Backends](../configuration/backends-dicom-storage.md#dicom-scu-service-class-user) for the options and the
C-STORE request body.

### Endpoint Usage (SCP - Service Class Provider)

When configured as an endpoint, the DICOM service accepts DICOM network connections via **DimseAdapter**. Inbound DIMSE is converted to `RequestEnvelope` and processed by the unified `PipelineExecutor`.

```toml
[endpoints.dicom_scp]
service = "dicom"

[endpoints.dicom_scp.options]
local_aet = "HARMONY_SCP"
# bind_addr = "0.0.0.0"
# port = 11112
```

**How it works (Phase 6)**:
1. **DimseAdapter** is automatically spawned by the orchestrator (`src/lib.rs::run()`) when a pipeline references a DICOM endpoint
2. Inbound DIMSE requests (C-FIND, C-STORE, etc.) are converted to `RequestEnvelope`
3. **PipelineExecutor** processes the envelope through middleware and backends
4. Response is converted back to DIMSE and sent to the calling AET

**Failure statuses (C-FIND, C-GET, C-MOVE)**:

When the pipeline fails, the SCP answers with a failure status. A failure never reaches the client as Success with zero matches. A request that matched nothing is not a failure: PS3.4 answers it with Success and zero results or sub-operations, so the client can tell an empty result from an outage. The response carries only the status code, never error text. The full error goes to the server log at error level, because it often names hosts, ports and AE titles.

| Cause | C-FIND | C-GET and C-MOVE |
|-------|--------|------------------|
| Backend answers HTTP 500, for example a `dicom_scu` backend that cannot reach its PACS | `0x0110` Processing failure | `0x0110` |
| Backend answers HTTP 408 or 502 to 504, a timeout, or the bridge middleware reports an upstream failure | `0xA700` Refused: Out of resources | `0xA701` Refused: Out of resources, unable to calculate matches |
| Backend answers HTTP 429, or a `policies` rate limit refuses the request | `0xA700` | `0xA702` Refused: Out of resources, unable to perform sub-operations |
| Backend answers HTTP 401 or 403, or a `policies` rule denies the client | `0x0124` Refused: Not authorized | `0x0124` |
| Backend answers HTTP 2xx, but the body has `"success": false` at the top level | `0x0110` | `0x0110` |
| Middleware raises `DicomObjectNotFound` | `0x0000` Success, no matches | `0x0000` Success, zero sub-operations |
| The `dicom_scu` backend finds no study for a C-MOVE | Not applicable | `0x0000` Success, zero sub-operations |
| Backend answers HTTP 404 or 410 for any other reason | `0xC000` | `0xC000` |
| A streamed result fails or answers 404 after it started | Failure. Matches already sent stay sent, and the final status is the failure | Failure for that sub-operation |
| Any other error with no status, such as a missing pipeline | `0xC000` | `0xC000` |

`src/adapters/dimse/status_mapper.rs` maps HTTP statuses and pipeline errors to DIMSE statuses. The DIMSE adapter then adjusts the result for the operation. PS3.4 defines `0xA701` and `0xA702` only for C-GET and C-MOVE, so a C-FIND gets `0xA700` instead. C-GET and C-MOVE have no `0xA700`, so they get `0xA701` in its place. `0xA801` means "Move Destination unknown" and is defined only for C-MOVE, so a pipeline failure never sends it. It gets `0xC000`.

Only typed signals make an empty result: `DicomObjectNotFound`, or the `dicom_study_not_found` response metadata the `dicom_scu` backend sets when it finds no study to move. A bare HTTP 404 is a failure, because it can also mean a denial (`path_filter` answers 404) or a wrong `base_url`. A query that matches nothing answers 2xx with an empty list. Error text is never read for this, because it can hold remote status codes, ports and instance counts.

The SCP sends C-MOVE requests to the pipeline as retrievals, so C-GET and C-MOVE share their codes. A C-MOVE to a destination AE title that is not in the SCP's AE table still gets `0xA801` before the pipeline runs. When every sub-operation fails, C-GET and C-MOVE answer `0xA702`. When some succeed and some fail, they answer `0xB000`.

**C-STORE acknowledgement**: the SCP acknowledges a store only when the pipeline's final HTTP status is 2xx, `dicom_status` (when set) is `0x0000` or a warning, and the body does not have `"success": false`. Any other result is a failure, because a sender may delete its copy once a store is acknowledged.

**HTTP→DICOM bridge**: Use an HTTP endpoint with a DICOM backend (SCU) in the same pipeline (see below).

## Usage Examples

### Backend SCU Operations

HTTP requests to backends configured with DICOM service will trigger DIMSE operations. The DICOM SCU backend uses `resolve_operation()` to determine the DIMSE action from the `Command` on the envelope (with fallback to path-based detection).

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
- **Native DIMSE SCP**: Endpoint (SCP) implementation uses `dicom-ul` Rust library (no external dependencies)
- **Native SCU**: Backend (SCU) operations C-ECHO, C-FIND, C-STORE and C-GET use `dicom-ul`. C-GET negotiates the SCP role for the storage SOP classes and streams each instance as it is received. C-MOVE still uses the DCMTK `movescu` tool.
- **SCP Operations**: C-ECHO, C-FIND, C-STORE supported via native Rust implementation
- **Persistent Store SCP**: Supports persistent Store SCP mode for C-STORE delivery
- **Dual Service Support**: Single service type supports both backend and endpoint usage
- **Configuration Integration**: Seamlessly integrated with existing service architecture
- **Streaming**: C-GET, C-MOVE and C-FIND results stream through the SCP as the pipeline produces them; the `QueryProvider` trait returns streams
- **Validation**: Proper configuration validation for both usage patterns

### 📋 Planned Enhancements
1. **TLS Support**: Secure DICOM connections for SCU/SCP
2. **Hardening & Observability**: Enhanced error handling, metrics, and logs across DIMSE flows
3. **Performance Optimization**: Connection pooling and association reuse

## Configuration Examples

See `examples/default/pipelines/dimse-integration.toml` for a complete configuration demonstrating both backend (SCU) and endpoint (SCP) usage patterns.

## Notes and Tips

- DCMTK requirement: only C-MOVE on the SCU side shells out to DCMTK (`movescu`). Ensure DCMTK is installed and on PATH before using C-MOVE.
- DCMTK CLI tag format: when passing keys via `-k`, use plain tag form `gggg,eeee` without parentheses. For example, set the QueryRetrieveLevel and keys like:
  - `-k 0008,0052=STUDY`
  - `-k 0020,000D=<StudyInstanceUID>`
  - `-k 0010,0020=<PatientID>`
- C-MOVE destination listener: `movescu` must listen for incoming C-STORE. Harmony config exposes `incoming_store_port`; the SCU uses DCMTK’s `+P <port>` option and `-aem <DEST_AET>`. Ensure the QR SCP’s HostTable includes the destination AET with the same host/port.
- Test artifacts under `./tmp`:
  - C-FIND: `./tmp/dcmtk_find_<uuid>/rsp*.dcm`
  - C-GET:  none unless an output directory is given; the `dicom_scu` backend writes `./tmp/dimse/<folder>/<SOPInstanceUID>.dcm`
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

- C-MOVE produces no files even though DCMTK reports success
  - DCMTK may write files without a `.dcm` extension (e.g., `SC.<SOPInstanceUID>`). Tools or code expecting only `.dcm` names might miss them.
  - In Harmony, we stream back all files in the DCMTK output directory, regardless of extension.
  - C-GET does not use DCMTK. With an output directory, each instance is written as `<SOPInstanceUID>.dcm`.

- DCMTK `-k` key errors like "bad key format"
  - Use `gggg,eeee` format without parentheses. Examples: `0008,0052=STUDY`, `0020,000D=<UID>`, `0010,0020=<PatientID>`.

- Duplicate or quota warnings in dcmqrscp logs
  - dcmqrscp may delete stored files due to duplicate SOP Instance UID or internal quotas while still indexing the dataset. This can be normal for tests and not an error.

- Debugging tips
  - Run pre-MOVE diagnostics with `findscu` at STUDY level and `-X -od` to see matched identifiers.
  - Set `HARMONY_TEST_DEBUG=1` to include `movescu_last.json` in API responses for MOVE.
