# Reverse HTTP/3 Tunnels Architecture

## Executive Summary

Based on the current architecture of `harmony-proxy`, **it is highly feasible to support reverse HTTP/3 tunnels**, and the existing architectural patterns are actually extremely well-suited for this feature.

## Architectural Alignment

The current codebase contains several structural advantages that make implementing reverse tunnels straightforward:

### 1. Underlying Transport (`quinn` + `tokio`)
The codebase already natively embeds the `quinn` crate (a pure-Rust, fully asynchronous QUIC implementation) and `tokio`. 
* In a reverse tunnel scenario (like Cloudflare Tunnel or ngrok), the proxy must act as a client that dials out to a central coordinator. 
* Because QUIC streams are symmetric after the initial handshake, the proxy can establish an outbound `quinn::Connection` and immediately start listening for incoming streams initiated by the remote server using `connection.accept_bi().await`. This entirely bypasses the traditional client/server asymmetry of TCP-based HTTP without requiring complex WebSocket upgrades.

### 2. Protocol-Agnostic Pipeline Execution
The core of the business logic routing, middleware, and backend dispatching is encapsulated in `PipelineExecutor::execute`. 
* The executor takes an abstracted `RequestEnvelope<Vec<u8>>` and returns a `ResponseEnvelope<Vec<u8>>`. 
* It does not care where the bytes came from. A reverse tunnel integration simply needs to read bytes from an accepted QUIC stream, construct an envelope, pass it to the executor, and write the response back down the stream. There is no need to rewrite or duplicate any routing, middleware, or backend code.

### 3. The `ProtocolAdapter` Paradigm
The system is built around extensible `ProtocolAdapter`s (e.g., `HttpAdapter`, `Http3Adapter`, `DimseAdapter`). 
While current adapters are built as *inbound listeners* that bind to a local socket (`bind_addr`), the trait lifecycle is managed via a `tokio::task::JoinHandle` and a `CancellationToken`. 
* A `ReverseHttp3Adapter` could be implemented. Instead of binding to a port, its `start` method would instantiate a `quinn::Endpoint` client, connect to the upstream tunnel server, and loop over `accept_bi()` or handle HTTP/3 WebTransport sessions. 
* It would automatically benefit from the application's graceful shutdown and configuration reloading mechanisms.

## Implementation Strategy

To implement this capability, the following additions would be required:

### 1. Reverse Tunnel Configuration
A new configuration block in `config.toml` (e.g., `[tunnels]`) specifying:
* The target coordinator URL.
* The pipelines to expose over the tunnel.
* Authentication credentials (like tokens or mTLS certificates).

### 2. Tunnel Adapter Implementation
A new adapter (e.g., `src/adapters/tunnel/mod.rs` or `src/adapters/reverse_http3/mod.rs`) that:
* Acts as a QUIC client dialing out to the coordinator.
* Implements a robust reconnect loop (with exponential backoff) in case the connection drops.
* Spawns a new `tokio::task` for every `(SendStream, RecvStream)` accepted from the coordinator.
* Implements the `ProtocolAdapter` trait so it can be managed by the `AdapterRegistry`.

### 3. Stream Semantic Choice
* **Raw QUIC:** Drop strict HTTP/3 semantics over the tunnel and just use raw `quinn` streams for the multiplexing. This is the simplest and highly performant approach.
* **Strict HTTP/3:** If the tunnel coordinator expects strict HTTP/3, use the existing `h3` dependency combined with WebTransport extensions (which allow servers to initiate streams to a connected client) or MASQUE (CONNECT-UDP / CONNECT-IP) primitives.
