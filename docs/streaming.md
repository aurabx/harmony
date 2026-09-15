# DICOM Streaming Architecture

## Overview

Harmony's streaming architecture enables efficient processing of large DICOM studies by treating each instance as an independent unit that flows through the pipeline. This eliminates memory bottlenecks, reduces latency, and enables progressive processing.

**Key Benefits:**
- **Constant Memory**: Process 1000+ instance studies with ~2MB memory overhead per instance
- **Low Latency**: First instance available within milliseconds of backend response
- **Progressive Processing**: Middleware operates on each instance independently
- **Fault Tolerance**: Per-instance error handling, failed instances don't block the stream

## Architecture

### Components

```
┌─────────────────────────────────────────────────────────────────┐
│                     STREAMING PIPELINE                           │
│                                                                  │
│  ┌──────────┐    ┌─────────────┐    ┌─────────────┐           │
│  │ Endpoint │───▶│ Middleware  │───▶│  Streaming  │           │
│  │          │    │  (Incoming) │    │   Backend   │           │
│  └──────────┘    └─────────────┘    └──────┬──────┘           │
│                                              │                   │
│                                              ▼                   │
│                                      Stream<Envelope>            │
│                                              │                   │
│                                              ▼                   │
│                         ┌────────────────────────────┐          │
│                         │  Per-Instance Processing   │          │
│                         │  ├─ Middleware (Outgoing) │          │
│                         │  └─ Endpoint Response     │          │
│                         └────────────────────────────┘          │
└─────────────────────────────────────────────────────────────────┘
```

### Flow

1. **Request Phase**: Single request envelope processed through endpoint and incoming middleware
2. **Backend Streaming**: Backend yields `Stream<ResponseEnvelope>` - one per DICOM instance
3. **Per-Instance Processing**: Each envelope independently through outgoing middleware
4. **Response Assembly**: Endpoint converts stream to appropriate protocol format

## Services

### Streaming Backends

#### dicom_scu_streaming

Streams DICOM instances from a PACS using C-GET or C-FIND operations.

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

## Pipeline Configuration

### Basic Streaming Pipeline

```toml
[pipelines.dicom_streaming]
description = "Stream DICOM instances with progressive processing"
networks = ["default"]
endpoints = ["http_api"]
backends = ["pacs_streaming"]
middleware = ["auth", "transform"]
```

**Execution Flow:**
1. HTTP request received on `http_api` endpoint
2. Request through `auth` middleware (incoming direction)
3. Request to `pacs_streaming` backend → returns Stream
4. **For each instance:**
   - Apply `auth` middleware (outgoing direction)
   - Apply `transform` middleware (outgoing direction)
   - Convert to HTTP stream format
5. HTTP response streamed back to client

### Bidirectional Streaming

**Proxy A (Sender):**
```toml
[endpoints.dicom_listener]
service = "dicom_scp"  # DIMSE endpoint

[backends.local_pacs]
service = "dicom_scu_streaming"

[endpoints.http_stream]
service = "http_streaming"

[pipelines.sender]
endpoints = ["dicom_listener"]
backends = ["local_pacs"]
# Streams to http_stream endpoint
```

**Proxy B (Receiver):**
```toml
[endpoints.http_api]
service = "http"

[backends.remote_stream]
service = "http_streaming_backend"  # Consumes stream from Proxy A

[backends.target_pacs]
service = "dicom_scu"

[pipelines.receiver]
endpoints = ["http_api"]
backends = ["remote_stream", "target_pacs"]
# Each instance forwarded to target_pacs
```

## Middleware Processing

### Per-Instance Middleware

All middleware operates **independently** on each streamed instance:

```toml
[middleware.transform_dicom]
type = "transform"

[middleware.transform_dicom.options]
spec_path = "transforms/anonymize.json"
apply = "right"  # Apply to backend responses
fail_on_error = false  # Continue stream on transform error
```

**Behavior:**
- Each ResponseEnvelope processed sequentially
- Transform applied to `normalized_data`
- Pre-transform snapshot available in `normalized_snapshot`
- Failed transforms logged, stream continues if `fail_on_error = false`

### Transform Examples

#### Extract Patient Demographics
```json
[
  {
    "operation": "shift",
    "spec": {
      "00100010": "patient.name",
      "00100020": "patient.id",
      "00100030": "patient.dob"
    }
  }
]
```

#### Anonymize DICOM Tags
```json
[
  {
    "operation": "remove",
    "spec": {
      "00100010": "",
      "00100020": "",
      "00100030": ""
    }
  },
  {
    "operation": "default",
    "spec": {
      "00100010": {"vr": "PN", "Value": ["ANONYMOUS"]},
      "00100020": {"vr": "LO", "Value": ["ANON-ID"]}
    }
  }
]
```

## Use Cases

### 1. Progressive Study Retrieval

**Scenario**: Retrieve 500-instance CT study from remote PACS

**Traditional Approach**: 
- Buffer entire 1GB study in memory
- Wait 30+ seconds before first instance available
- High memory usage, long latency

**Streaming Approach**:
```toml
[pipelines.progressive_retrieve]
endpoints = ["http_api"]
backends = ["remote_pacs_streaming"]
middleware = []
```

**Benefits**:
- First instance available in <100ms
- Constant 2MB memory per instance
- Total memory: ~10MB (5 instances in flight)

### 2. Cross-Site Study Transfer

**Scenario**: Transfer studies between hospitals over WAN

```
Hospital A → Proxy A → HTTP/3 → Proxy B → Hospital B
```

**Proxy A Configuration**:
```toml
[endpoints.dicom_scp_a]
service = "dicom_scp"
local_aet = "HOSPITAL_A"

[backends.local_pacs_a]
service = "dicom_scu_streaming"

[endpoints.http_stream_a]
service = "http_streaming"
format = "ndjson"

[middleware.anonymize_outbound]
type = "transform"
spec_path = "transforms/anonymize-phi.json"
apply = "right"
```

**Proxy B Configuration**:
```toml
[endpoints.http_api_b]
service = "http"

[backends.stream_from_a]
service = "http_streaming_backend"
base_url = "https://proxy-a.hospital-a.org:8443"

[backends.local_pacs_b]
service = "dicom_scu"

[middleware.restore_metadata]
type = "transform"
spec_path = "transforms/restore-local-ids.json"
apply = "right"
```

**Benefits**:
- Secure HTTP/3 transport (TLS 1.3)
- Per-instance PHI anonymization
- Progressive transfer (no study buffering)
- Middleware processing during transfer

### 3. Real-Time Study Processing

**Scenario**: AI inference on incoming DICOM instances

```toml
[pipelines.ai_inference]
endpoints = ["dicom_scp"]
backends = ["ai_backend_streaming"]
middleware = ["preprocess", "ai_transform"]

[middleware.preprocess]
type = "transform"
spec_path = "transforms/normalize-pixels.json"
apply = "right"

[middleware.ai_transform]
type = "transform"
spec_path = "transforms/add-inference-results.json"
apply = "right"
```

**Flow**:
1. Instance received via C-STORE
2. Pixel normalization applied
3. AI inference (external service)
4. Results appended to DICOM metadata
5. Streamed to destination PACS

## Performance

### Benchmarks

**Environment**: 
- 2023 MacBook Pro M3 Max, 64GB RAM
- Local Orthanc PACS
- 500-instance CT study, 2MB avg per instance

**Traditional (Buffered)**:
- Memory: 1.2GB peak
- Latency: 28s until first instance
- Throughput: ~40 instances/sec

**Streaming**:
- Memory: 12MB peak (6 instances in flight)
- Latency: 80ms until first instance
- Throughput: ~120 instances/sec

**Streaming + Middleware**:
- Memory: 15MB peak
- Latency: 95ms until first instance
- Throughput: ~100 instances/sec

### Memory Characteristics

```
Memory Usage Over Time (500 instances)

Traditional:
    1.2GB ┤     ╭────────╮
          │     │        │
    600MB ┤     │        │
          │     │        │
      0MB ┼─────╯        ╰─────
          └──────────────────────▶ Time (30s)

Streaming:
     15MB ┤ ╭─╮ ╭─╮ ╭─╮ ╭─╮ ╭─╮
          │ │ │ │ │ │ │ │ │ │ │
     10MB ┤ │ │ │ │ │ │ │ │ │ │
          │ │ │ │ │ │ │ │ │ │ │
      0MB ┼─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─▶ Time (4s)
          └──────────────────────▶
```

### Tuning

**Concurrency Control** (TODO):
```toml
[backends.pacs_streaming.options]
max_concurrent_instances = 5  # Default: 10
instance_buffer_size = 1      # Default: 1 (no buffering)
```

**Backpressure Handling**:
- `dicom_scu_streaming` C-GET: automatic. The backend answers each C-STORE
  sub-operation only after the instance has been accepted downstream, and holds
  at most 8 instances ahead of the consumer. A slow client therefore slows the
  PACS instead of growing memory on the proxy.
- `http_streaming_backend`: the HTTP response body is read as the consumer
  pulls, so a slow client applies TCP backpressure to the upstream proxy.

## Error Handling

### Per-Instance Errors

Failed instances are logged and skipped. Stream continues.

**Log Output**:
```
WARN Instance 47 failed transform: Invalid DICOM tag 00111111
INFO Continuing stream, 46 successful, 1 failed
```

**Error Envelope** (SSE format):
```
event: error
data: {"_sequence":47,"_error":"Transform failed","_sop_instance_uid":"1.2.3..."}
```

### Stream Termination

Stream terminates on:
- Backend connection failure
- Client disconnect
- Unrecoverable protocol error

**Log Output**:
```
ERROR Backend connection lost: Connection reset by peer
INFO Stream terminated after 123 instances (120 successful, 3 failed)
```

### Retry Logic (TODO)

Automatic retry for:
- Transient network errors
- Backend timeouts
- HTTP 5xx responses

```toml
[backends.pacs_streaming.options]
retry_attempts = 3
retry_delay_ms = 1000
retry_backoff_multiplier = 2.0
```

## Testing

### Unit Tests

Located in `src/models/services/types/`:
- `dicom_scu_streaming_test.rs`
- `http_streaming_test.rs`
- `http_streaming_backend_test.rs`

```bash
cargo test --lib streaming
```

### Integration Tests

Located in `tests/streaming_integration.rs`:

```bash
# Full streaming test suite
cargo test --test streaming_integration

# Specific test
cargo test --test streaming_integration test_dicom_to_http_stream
```

### Manual Testing

**Start Streaming Proxy**:
```bash
cargo run -- --config examples/dicom-streaming/bidirectional-streaming.toml
```

**Trigger Stream (C-GET)**:
```bash
# Using dcmtk tools
getscu -v -aet HARMONY_CLIENT -aec SENDER_SCP localhost 11112 \
  -k "StudyInstanceUID=1.2.840.113619.2.55.3.2609295819.820.1299003904.1"
```

**Monitor Stream (HTTP)**:
```bash
curl -N http://localhost:8080/dicom-stream \
  -H "Accept: application/x-ndjson" \
  -d '{"StudyInstanceUID":"1.2.840..."}'
```

**Expected Output**:
```json
{"_sequence":1,"_sop_instance_uid":"1.2.3.4.5.6.7.1","00080016":{"vr":"UI"...}}
{"_sequence":2,"_sop_instance_uid":"1.2.3.4.5.6.7.2","00080016":{"vr":"UI"...}}
...
```

## Troubleshooting

### "No instances streamed"

**Symptom**: Stream completes but no instances received

**Causes**:
1. Query returned no results
2. Backend not configured as streaming service
3. Middleware rejecting all instances

**Debug**:
```bash
RUST_LOG=harmony=debug,harmony::models::services::types::dicom_scu_streaming=trace cargo run
```

**Check Logs For**:
```
DEBUG Started DICOM C-GET operation
DEBUG Received instance 1: 2048 bytes
ERROR Transform middleware rejected instance (missing required field)
```

### "Memory still growing"

**Symptom**: Memory usage increases with study size despite streaming

**Causes**:
1. Using non-streaming backend (`dicom_scu` instead of `dicom_scu_streaming`)
2. Middleware buffering instances (custom middleware issue)
3. A backend or endpoint that collects results before returning them

**Verify Streaming Backend**:
```bash
grep -A5 "backends.pacs" config.toml
# Should show: service = "dicom_scu_streaming"
```

### "Stream hangs after N instances"

**Symptom**: Stream stops progressing mid-study

**Causes**:
1. Backend connection timeout
2. Middleware deadlock (unlikely)
3. Client disconnect not detected

**Increase Timeout**:
```toml
[backends.pacs_streaming.options]
connection_timeout_secs = 300  # Default: 60
```

### "Transform errors breaking stream"

**Symptom**: Stream terminates on first transform error

**Cause**: `fail_on_error = true` in middleware config

**Solution**:
```toml
[middleware.my_transform.options]
fail_on_error = false  # Continue on transform errors
```

## Future Enhancements

### Planned Features

1. **Backpressure Control**: Automatic flow control when client consumes slowly
2. **Adaptive Batching**: Group small instances for efficiency
3. **Parallel Streaming**: Multiple backend connections for higher throughput
4. **Resume Support**: Stream continuation after connection interrupts
5. **Stream Multiplexing**: Multiple concurrent streams over single HTTP/3 connection

### Experimental Features

**Chunked Instance Streaming**: Stream large instances in chunks

```toml
[backends.pacs_streaming.options]
enable_chunked_instances = true
chunk_size_bytes = 65536  # 64KB chunks
```

**Benefits**: 
- Sub-instance progressive processing
- Lower memory for huge instances (500MB+ raw pixel data)

## References

- [Envelope Architecture](envelope.md)
- [Pipeline Executor](router.md)
- [Protocol Adapters](adapters.md)
- [DICOM Services](backends.md#dicom-scu--dicom-scp)
- [HTTP Streaming Standards](https://html.spec.whatwg.org/multipage/server-sent-events.html)
- [NDJSON Format](http://ndjson.org/)
