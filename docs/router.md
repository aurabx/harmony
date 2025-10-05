# Router Description

The router is responsible for routing incoming and outgoing requests through a structured series of layers, ensuring proper authentication, transformation, and communication with relevant services and backends. This document outlines the expected behavior of the router.

## Routing Structure

The router processes requests via a pipeline of components:

1. **Endpoint**
   - Entry/exit point for HTTP requests and responses.
2. **Middleware**
   - A single, ordered chain applied between endpoint and backend. Use it for authentication, transformations, logging, header injection, etc.
3. **Backend**
   - Communicates with external third-party services or systems.

## Behavior

### Incoming Requests

1. The request is received by the router for a specified path.
2. The router identifies the **Endpoint** based on the configured `path_prefix`.
3. The router selects the first matching pipeline for that endpoint.
4. The router executes the pipeline:
   a. Applies the configured `middleware` list in order (auth, transforms, logging, etc.).
   b. Forwards the request to the configured `backend`.
5. The **Backend** communicates with the external system and returns the response.
6. The response follows the reverse path and is sent to the client.

### Outgoing Requests

1. The application initiates an outgoing request for a specific backend.
2. The router applies the same `middleware` chain semantics (if configured for the path/pipeline) and sends the request to the backend.
3. The backend returns a response, which can pass through the middleware chain on the way back, then back to the caller.

## Implementation Notes

- Middleware: A single ordered list, applied between endpoint and backend. The same ordering principle applies to response handling (reverse, if applicable).
- Dynamic Routing: Routes are dynamically configured based on the `network_name` and validated at startup.
- Error Handling: Missing middleware/backend configurations should result in appropriate HTTP-level errors, such as `NOT_FOUND` or `BAD_REQUEST`.


## Pipeline Flow Overview

Endpoint
  -> Middleware (auth, transforms, logging, etc.)
  -> Backend

The response follows the reverse order back to the client.

## Concepts

- Endpoint
  - Entry/exit point converting HTTP requests/responses to/from internal Envelopes.

- Middleware
  - Single ordered chain that augments the flow (auth, transforms, logging, header injection).

- Backend
  - Outbound connector to external services; converts Envelopes to protocol-specific requests.

Notes
- Place authentication early in the middleware list to fail fast.
- Arrange transforms where required context and normalized data are available.
