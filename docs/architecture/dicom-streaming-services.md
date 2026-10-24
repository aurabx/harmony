---
title: DICOM Streaming Services
description: The endpoint and backend services that take part in DICOM streaming.
status: active
updated: 2026-10-24
type: Architecture
related:
  - architecture/dicom-streaming.md
---

# DICOM Streaming Services

The endpoint and backend services that take part in DICOM streaming. This page is part of [DICOM Streaming Architecture](dicom-streaming.md).

## Services

### Streaming Backends

#### dicom_scu_streaming

Streams DICOM instances from a PACS using C-GET or C-FIND operations, and sends instances to a PACS using C-STORE.

**Configuration:**
```toml
[backends.pacs_streaming]
service = "dicom_scu_streaming"

[backends.pacs_streaming.options]
aet = "REMOTE_PACS"              # Remote AE Title
host = "pacs.example.com"        # PACS hostname
port = 4242                       # PACS port
local_aet = "HARMONY_SCU"        # Local AE Title
dimse_retrieve_mode = "get"      # "get" (C-GET) or "find" (C-FIND)
use_tls = false                   # Enable TLS (default: false)
```

**Operations:**
- **C-GET**: Recommended for streaming. The backend runs the C-GET itself on
  dicom-ul, so each instance is yielded the moment its C-STORE sub-operation
  has been received, before the retrieval is complete. Nothing is written to
  disk. At most 8 instances wait in memory ahead of the consumer; beyond that
  the backend stops answering the PACS until the consumer catches up.
- **C-FIND**: Queries only (metadata). Instances yielded as query results.
- **C-STORE**: Sends one instance to the PACS per request. See
  [C-STORE requests](#c-store-requests).

##### C-STORE requests

The request must carry the instance as a DICOM Part 10 file. Two forms are
accepted:

| Form | Body | Sent by |
|------|------|---------|
| Part 10 body | The Part 10 file as the whole body | `dicom_to_http` on another Harmony, or any HTTP client |
| JSON body | A JSON object with the Part 10 file as base64 in `part10_b64` | The `dicom_scp` endpoint in the same pipeline |

The backend recognises a Part 10 body by its `DICM` code. It does not read
`Content-Type`.

Any other body is rejected, including raw data set bytes without the Part 10
header. The backend never opens a file that a request names. A JSON body with
a `file` key and no `part10_b64` is rejected. The HTTP client gets a 502 with
an empty body, and the reason is in the Harmony log.

The transfer syntax is read from the file meta group of the Part 10 file.

To bridge a C-STORE to another Harmony, use `dicom_to_http` with an
`http_streaming_backend`. The request body is then the Part 10 file, sent with
these headers:

| Header | Value |
|--------|-------|
| `Content-Type` | `application/dicom` |
| `x-dimse-op` | `store` |
| `x-dicom-transfer-syntax` | Transfer Syntax UID of the file |
| `x-dicom-sop-class-uid` | SOP Class UID, when known |
| `x-dicom-sop-instance-uid` | SOP Instance UID, when known |

The backend takes the transfer syntax and both SOP UIDs from the file. It
rejects a request in which one of the three `x-dicom-*` headers is present and
differs from the file.

`dicom_to_http` with the `http` backend does not bridge a C-STORE. That
backend does not send the headers above, and the request body stays the JSON
that `dicom_scp` builds.

##### C-STORE response

Behind an `http_streaming` endpoint the answer to a store request is one
NDJSON line under HTTP 200. The HTTP status is 200 whether or not the PACS
took the instance. Read `success` to learn the result.

```json
{"operation":"store","success":false,"remote_aet":"PACS","host":"pacs.example.com","port":4242,"sop_instance_uid":"1.2.3","error":"C-STORE to PACS@pacs.example.com:4242 failed: could not connect to server","_stream_sequence":1}
```

| Key | Value |
|-----|-------|
| `success` | `true` when the PACS accepted the instance, otherwise `false` |
| `error` | The reason, present only when `success` is `false` |
| `remote_aet`, `host`, `port` | The PACS the backend sent to |
| `sop_class_uid`, `sop_instance_uid` | Taken from the file, when known |

A `dicom_scp` endpoint that bridged the C-STORE reads this line. When
`success` is `false`, it answers the sending device with status `0xC000`, so
the device keeps the instance and can send it again. The `error` text goes to
the Harmony log. A response-side transform on either Harmony must keep the
`success` key, or a failed store is reported as stored.

**Transfer syntaxes:** the `transfer_syntaxes` option lists the transfer
syntaxes proposed for the storage presentation contexts of the C-GET, most
preferred first. Instances arrive in one of them. The default is Explicit VR
Little Endian and Implicit VR Little Endian.

```toml
[backends.pacs_streaming.options]
transfer_syntaxes = ["1.2.840.10008.1.2.1", "1.2.840.10008.1.2"]
```

**Envelope Structure:**
- `original_data`: Raw DICOM bytes
- `normalized_data`: JSON representation with tag hierarchy
- `metadata`: Contains `sop_instance_uid`, `stream_sequence`, DICOM-specific fields

#### http_streaming_backend

Consumes HTTP streams from another Harmony proxy or compatible service.

**Configuration:**
```toml
[backends.remote_stream]
service = "http_streaming_backend"

[backends.remote_stream.options]
base_url = "https://remote-proxy:8443"
format = "auto"  # "sse", "ndjson", "multipart", or "auto"
request_timeout_secs = 300
enable_compression = false
```

**Format Auto-Detection:**
- Reads `Content-Type` header from response
- Falls back to NDJSON if ambiguous
- Supports SSE, NDJSON, and Multipart formats

**Behavior:**
- Establishes HTTP connection to remote endpoint
- Parses stream format based on Content-Type
- Reconstructs ResponseEnvelopes with sequence tracking
- Handles connection interrupts with automatic retry (TODO)

### Streaming Endpoints

#### http_streaming

Converts ResponseEnvelope streams to HTTP streaming formats.

**Configuration:**
```toml
[endpoints.http_stream]
service = "http_streaming"

[endpoints.http_stream.options]
format = "ndjson"  # "sse", "ndjson", "multipart", or "auto"
enable_compression = false
```

Routes `path_prefix` (default `/`) and every path below it, for GET, HEAD, POST, PUT, PATCH, DELETE and OPTIONS. See [endpoints.md](../configuration/endpoints.md#http-passthru) for why TRACE and CONNECT are excluded.

**Formats:**

##### NDJSON (Recommended)
```
Content-Type: application/x-ndjson

{"_sequence":1,"_sop_instance_uid":"1.2.3...","data":{...}}
{"_sequence":2,"_sop_instance_uid":"1.2.4...","data":{...}}
```

- One JSON object per line
- Most efficient for programmatic consumption
- Easy to parse and debug

##### SSE (Server-Sent Events)
```
Content-Type: text/event-stream

event: data
data: {"_sequence":1,"_sop_instance_uid":"1.2.3...","data":{...}}

event: data
data: {"_sequence":2,"_sop_instance_uid":"1.2.4...","data":{...}}
```

- Browser-compatible EventSource API
- Automatic reconnection support
- Best for web applications

##### Multipart
```
Content-Type: multipart/x-mixed-replace; boundary=harmonystream

--harmonystream
Content-Type: application/json
X-Stream-Sequence: 1

{"_sop_instance_uid":"1.2.3...","data":{...}}
--harmonystream
```

- Legacy compatibility
- Per-part headers for metadata
- Higher overhead than NDJSON

### Standard Endpoints/Backends

#### dicom_scp (Endpoint)

Receives DIMSE requests. C-GET, C-MOVE and C-FIND results are sent to the
client as the pipeline produces them: each C-STORE sub-operation goes out as
soon as its dataset arrives from the backend, so a large study never sits in
memory on the proxy.

C-GET and C-MOVE Pending responses carry the number of remaining
sub-operations, which the proxy does not know when the datasets come from a
streaming backend. Pending responses are therefore sent only when the total
is known up front, which the standard allows. The final response reports the
completed, warning and failed counts and never carries a remaining count.

```toml
[endpoints.dicom_listener]
service = "dicom_scp"

[endpoints.dicom_listener.options]
local_aet = "HARMONY_SCP"
port = 11112
enable_echo = true
enable_find = true
enable_get = true
```

#### dicom_scu (Backend)

Standard non-streaming DICOM backend. Buffers entire response. Use `dicom_scu_streaming` for large studies.

```toml
[backends.pacs]
service = "dicom_scu"

[backends.pacs.options]
aet = "PACS"
host = "localhost"
port = 4242
```
