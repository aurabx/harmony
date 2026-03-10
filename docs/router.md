# Router and Pipeline Architecture

**Last Updated**: 2026-02-08

Harmony uses a protocol-agnostic adapter architecture. This document describes the unified pipeline that handles all protocols (HTTP, DIMSE, HL7, etc.).

The pipeline is responsible for processing requests through a structured series of layers, ensuring proper authentication, transformation, and communication with relevant services and backends. All protocols follow the same pipeline execution path via `PipelineExecutor`.

## Architecture Overview

### Protocol Adapters

Each protocol (HTTP, DIMSE, HL7, etc.) has a dedicated **Protocol Adapter** that:
- Listens for protocol-specific requests
- Converts requests to `ProtocolCtx` + `RequestEnvelope`
- Calls the unified `PipelineExecutor`
- Converts `ResponseEnvelope` back to protocol-specific responses

**Available Adapters**:
- `HttpAdapter` - HTTP/HTTPS requests via Axum
- `DimseAdapter` - DICOM DIMSE (C-FIND, C-STORE, C-MOVE, C-ECHO)
- Future: HL7 MLLP, SFTP, MQTT, etc.

**For detailed information on implementing new adapters, see [adapters.md](adapters.md).**

### Pipeline Components

The `PipelineExecutor` processes requests via a pipeline of components:

1. **Endpoint**
   - Entry/exit point for HTTP requests and responses.
2. **Middleware**
   - A single, ordered chain applied between endpoint and backend. Use it for authentication, transformations, logging, header injection, etc.
3. **Backend**
   - Communicates with external third-party services or systems.

## Request Flow

### Complete Flow (All Protocols)

```
Protocol Request (HTTP/DIMSE/HL7/...)
  ↓
Protocol Adapter (HttpAdapter/DimseAdapter/...)
  ↓ Converts to
ProtocolCtx + RequestEnvelope
  ↓
PipelineExecutor (UNIFIED FOR ALL PROTOCOLS)
  ├─ 1. Endpoint Service Preprocessing (sets Command on envelope)
  ├─ 2. Incoming Middleware Chain (left) (may read/modify Command)
  ├─ 3. Backend Invocation (reads Command to decide protocol action)
  ├─ 4. Outgoing Middleware Chain (right)
  └─ 5. Endpoint Service Postprocessing
  ↓
ResponseEnvelope
  ↓ Converts back
Protocol Adapter
  ↓
Protocol Response (HTTP/DIMSE/HL7/...)
```

### Detailed Steps

1. **Protocol Adapter receives request**
   - For HTTP: Axum route handler in `HttpAdapter`
   - For DIMSE: SCP listener in `DimseAdapter`
   
2. **Adapter converts to protocol-agnostic format**
   - Creates `ProtocolCtx` with protocol-specific metadata
   - Builds `RequestEnvelope` via service
   
3. **PipelineExecutor processes the request**:
   a. **Endpoint preprocessing**: Service-specific request handling. The endpoint sets a typed `Command` on the envelope expressing the intended operation (e.g., Query, Retrieve, Store). See [commands.md](commands.md).
   b. **Incoming middleware** (left): Auth, transforms, logging (in order). Protocol-bridge middleware may read or modify the Command.
   c. **Backend invocation**: The backend reads the Command (via `resolve_operation()`) to decide what protocol action to perform (e.g., map `Query` to C-FIND, or to HTTP GET).
   d. **Outgoing middleware** (right): Response transforms, logging (in order)
   e. **Endpoint postprocessing**: Protocol-aware response shaping
   
4. **Adapter converts ResponseEnvelope back**
   - For HTTP: Axum `Response` with headers/body
   - For DIMSE: DICOM response PDUs with appropriate status codes

### Backend Communication

Backends are external services/systems that the pipeline communicates with:
- Can be HTTP endpoints, FHIR servers, DICOM PACS, databases, etc.
- Invoked during step 3 of the pipeline
- Return `ResponseEnvelope` with status, headers, and body
- If no backends configured, pipeline returns empty 200 OK response

## Implementation Details

### Module Structure

```
src/
├── adapters/              ← Protocol-specific I/O
│   ├── http/             ← HTTP adapter (Axum)
│   ├── dimse/            ← DIMSE adapter (DICOM SCP)
│   └── mod.rs            ← ProtocolAdapter trait
│
├── pipeline/             ← Protocol-agnostic execution
│   ├── executor.rs       ← PipelineExecutor (single source of truth)
│   └── mod.rs
│
├── router/               ← Configuration helpers
│   ├── route_config.rs   ← Route configuration types
│   └── mod.rs            ← Delegation to HttpAdapter
│
└── lib.rs                ← Orchestrator (spawns adapters)
```

### Key Files

- ✅ `src/pipeline/executor.rs` - Unified pipeline execution
- ✅ `src/adapters/http/` - HTTP protocol handling
- ✅ `src/adapters/dimse/` - DIMSE protocol handling
- ✅ `src/lib.rs::run()` - Spawns adapters per network

### Configuration

- **Networks**: Define bind addresses and protocol settings
- **Pipelines**: Link networks, endpoints, backends, and middleware
- **Endpoints**: Protocol entry points (HTTP paths, DIMSE AE titles, etc.)
- **Middleware**: Ordered list applied in both directions
- **Backends**: External service connectors

### Error Handling

- Missing configurations → 500 Internal Server Error
- Authentication failures → 401 Unauthorized
- Backend failures → 502 Bad Gateway
- Not found → 404 Not Found


## Pipeline Flow Summary

```
Protocol Adapter
  ↓
Endpoint Preprocessing (sets Command on envelope)
  ↓
Incoming Middleware (auth, transforms, may modify Command)
  ↓
Backend (reads Command to select protocol action)
  ↓
Outgoing Middleware (transforms, logging, etc.)
  ↓
Endpoint Postprocessing
  ↓
Protocol Adapter
```

**Key Principle**: All protocols use the same `PipelineExecutor`. Protocol-specific logic is isolated in adapters.

## Concepts

- **Envelope**
  - Protocol-agnostic data carrier. A `RequestEnvelope` carries the request, a `ResponseEnvelope` carries the response. See [envelope.md](envelope.md).

- **Command**
  - A typed description of operational intent (e.g., `Query`, `Retrieve`, `Store`, `Ping`) set on the `RequestEnvelope`. Endpoints produce commands; backends consume them. See [commands.md](commands.md).

- **Endpoint**
  - Entry/exit point converting protocol requests/responses to/from internal Envelopes. Sets a `Command` on the envelope during preprocessing.

- **Middleware**
  - Single ordered chain that augments the flow (auth, transforms, logging, header injection). Protocol-bridge middleware may read or modify the Command.

- **Backend**
  - Outbound connector to external services; reads the Command (via `resolve_operation()`) to decide what protocol action to perform.

Notes
- Place authentication early in the middleware list to fail fast.
- Arrange transforms where required context and normalized data are available.
