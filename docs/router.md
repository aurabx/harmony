# Router and Pipeline Architecture

**Last Updated**: 2026-09-24

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
   - For HTTP: `HttpAdapter` picks the endpoint from the network's route table (see [HTTP route matching](#http-route-matching))
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
│   ├── route_table.rs    ← HTTP route matching and conflict detection
│   └── mod.rs            ← Delegation to HttpAdapter
│
└── lib.rs                ← Orchestrator (spawns adapters)
```

### HTTP route matching

Each HTTP endpoint registers routes. An `http` endpoint registers its `path_prefix` and every path below it (`<path_prefix>/{*wildcard}`). Endpoints on one network may overlap. A collection endpoint at `/v1/checkout/sessions` and an item endpoint at `/v1/checkout/sessions/{id}/complete` both match `/v1/checkout/sessions/cs_1/complete`.

`HttpAdapter` gives axum one catch-all route and picks the endpoint itself, with `RouteTable` in `src/router/route_table.rs`:

1. Keep the routes whose path pattern matches the request path. If none match, the answer is `404 Not Found`.
2. Pick the most specific route. Compare the two patterns segment by segment from the left. At the first segment where they differ, a literal (`complete`) beats a parameter (`{id}`), and a parameter beats a catch-all (`{*wildcard}`). Between two parameters, or two catch-alls, the one with more literal text in front (`img_{id}` over `{id}`) wins.
3. If that route allows the method, its endpoint serves the request. A route that allows GET also serves HEAD. Otherwise the answer is `405 Method Not Allowed`, with an `Allow` header that lists that route's methods.

The method never sends a request to a less specific endpoint. Endpoints on one network can belong to different pipelines, with different auth, policies and backends. With a `jmix` endpoint whose routes start at `/api/jmix` and an `http` endpoint at `/api`, `DELETE /api/jmix/1` gets 405 from the JMIX route. It does not reach the `/api` pipeline.

For the UCP checkout shape:

| Request | Endpoint |
|---|---|
| `POST /v1/checkout/sessions` | `/v1/checkout/sessions` |
| `GET /v1/checkout/sessions/cs_1` | `/v1/checkout/sessions/{id}` |
| `PATCH /v1/checkout/sessions/cs_1` | `/v1/checkout/sessions/{id}` |
| `POST /v1/checkout/sessions/cs_1/complete` | `/v1/checkout/sessions/{id}/complete` |
| `GET /v1/checkout/sessions/cs_1/lines` | `/v1/checkout/sessions/{id}` |

Without the `{id}` endpoint, `GET /v1/checkout/sessions/cs_1` goes to the collection endpoint.

**Conflicts fail config load.** Two endpoints conflict when they register routes with the same shape. Same shape means the same segments, with parameter names ignored: `/v1/items/{id}` and `/v1/items/{sku}` have the same shape. The route is chosen by path alone, so this holds even when the two endpoints allow different methods. No request could be sent to one of them over the other. This also covers one endpoint listed in two pipelines on the same network. One endpoint may register the same shape more than once with different methods (the `jmix` endpoint does this for GET and POST on `/api/jmix`); those routes are merged. A route path that is not a valid pattern (for example `/v1/{id}.json`, or a path without a leading `/`) also fails the load. The error names the network, both endpoints, their pipelines and the files that define them:

```
ERROR: Invalid network 'default': route conflict: endpoint 'orders_a_in' (pipeline 'orders_a', defined in pipelines/orders-a.toml) and endpoint 'orders_b_in' (pipeline 'orders_b', defined in pipelines/orders-b.toml) both route '/v1/orders/{id}' and '/v1/orders/{order_id}'. ...
```

The check runs in `Config::validate`, so `--validate-config` and startup exit with code 1, and a hot reload with a conflict keeps the running config.

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
