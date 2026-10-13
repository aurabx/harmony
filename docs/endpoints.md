# Endpoints

**Last Updated**: 2026-09-24

Endpoints define protocol entry points into the Harmony pipeline. Each endpoint is associated with a **Service** that handles protocol-specific data conversion to/from the internal `Envelope` format.

> **Custom Services**: To create your own service types for endpoints, see [extensions.md](extensions.md#service-factories).

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
authentication = "hospital_a_auth"  # Bare key from [authentications]
```

A peer's `authentication` must be the bare key of an `[authentications]` entry. A reference that does not resolve stops the config from loading. See [Authentications](configuration.md#authentications).

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
- Sets a `Command` on the envelope based on the HTTP method (GET→Retrieve, POST→Store, PUT→Update, PATCH→Patch, DELETE→Delete, HEAD/OPTIONS→Ping). See [commands.md](commands.md).

**Methods**: GET, HEAD, POST, PUT, PATCH, DELETE and OPTIONS all reach the pipeline, on `path_prefix` and on every path below it. The endpoint does not restrict methods. Use a `method` rule in the policies middleware to limit them (see [policies-middleware.md](policies-middleware.md)). The `echo` and `http_streaming` endpoints accept the same methods.

TRACE and CONNECT are not routed and get `405 Method Not Allowed`. TRACE reflects the request, including credential headers, back to the client. CONNECT asks for a raw TCP tunnel, which a pipeline cannot carry.

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

**Paths**: the endpoint serves `path_prefix` and every path below it. `path_prefix` may contain parameters, such as `/v1/checkout/sessions/{id}/complete`. Endpoints on one network may overlap. The most specific endpoint serves each request, so an item endpoint at `/v1/orders/{id}` takes `/v1/orders/42` from a collection endpoint at `/v1/orders`. Two endpoints with the same path shape on one network fail config load. See [router.md](router.md#http-route-matching) for the full rule.

### FHIR

Extends the HTTP endpoint with FHIR-specific handling.

**Service behavior**:
- Accepts an HTTP request and converts it into a `RequestEnvelope`
- Provides FHIR-aware request/response handling
- Takes a `ResponseEnvelope` and converts it into a FHIR-compliant HTTP response
- Sets a `Command` based on FHIR interaction type: search→Query, read→Retrieve, create→Store, `_search`→Query, metadata→Ping, and others. See [commands.md](commands.md).

**Methods**: GET, HEAD, POST, PUT, PATCH, DELETE and OPTIONS on every path below `path_prefix`.

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
- Sets a `Command` based on method and route: GET-by-ID→Retrieve, GET-with-query→Query, POST→Store. See [commands.md](commands.md).

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
- Sets a `Command` based on DICOMweb route classification: QIDO-RS→Query, WADO-RS→Retrieve, STOW-RS→Store. See [commands.md](commands.md).
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
- Sets a `Command` from the DIMSE operation type (find→Query, get/move→Retrieve, store→Store, echo→Ping). See [commands.md](commands.md).
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
max_dataset_bytes = 4294967296  # Largest instance accepted, in bytes (default: 4 GiB)
```

`max_dataset_bytes` limits the data set of one DIMSE message. A C-STORE carries one instance per message, so it is a limit on the size of one instance. Harmony aborts the association when a peer sends more.

It is not a limit on memory. Harmony holds each instance in memory and copies it several times while it stores it, and every open association can hold one instance. The default accepts large breast tomosynthesis instances. Set the option to the largest instance you expect, and size the host for several times that value for each concurrent sender. Lower it on a host with little memory.

Endpoints that share a listen address use the largest value among them. An endpoint that does not set the option counts with the default, so the listener goes below 4 GiB only when every endpoint on it sets a lower value. Harmony logs a warning when the values differ.

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
  "command": {
    "message_id": 1,
    "sop_class_uid": "1.2.840.10008.5.1.4.1.1.2",
    "priority": "MEDIUM",
    "direction": "REQUEST"
  },
  "dataset": { "00100020": { "vr": "LO", "Value": ["PID1"] } },
  "file": "/path/to/storage/dimse/unique-id.dcm"
}
```

`dataset` is the instance as DICOM JSON. `file` is the path of the stored
Part 10 file on this host.

Two more keys are present when the pipeline's backend is `dicom_scu` (also
under its legacy name `dicom`) or `dicom_scu_streaming`, or when it is `http_streaming_backend` behind the
`dicom_to_http` middleware: `part10_b64`, the Part 10 file in base64, and
`transfer_syntax_uid`. The two DICOM backends send the instance from
`part10_b64` to the PACS. They never open the path in `file`. See
[backends.md](backends.md#dicom-scu-service-class-user) and
[streaming.md](streaming.md#c-store-requests).

This allows you to use middleware to process the file (e.g., extract metadata, forward to another system, or trigger a notification) after it has been safely stored.
