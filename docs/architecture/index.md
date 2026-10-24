# Architecture

How Harmony works today. This section describes the current system, not planned changes. Planned work lives in [plans](../plans/index.md).

- [System Description](overview.md): High-level overview of Harmony's components and how they fit into the Runbeam architecture.
- [Architecture Diagrams](diagrams.md): Diagrams of the unified request flow, data flow, and orchestration flow.
- [Router and Pipeline Architecture](router.md): How requests move from a protocol adapter through the PipelineExecutor and back.
- [Protocol Adapters](adapters.md): The ProtocolAdapter trait, the built-in HTTP, HTTP/3 and DIMSE adapters, and how to add a new one.
  - [Implementing a Protocol Adapter](adapters-implementing.md): How to implement a new protocol adapter, with best practices and common patterns.
- [Envelope: Data Exchange Format](envelope.md): The RequestEnvelope and ResponseEnvelope types that carry data through every pipeline.
  - [Envelope Types](envelope-types.md): Field reference for RequestEnvelope, ResponseEnvelope, and ProtocolCtx.
  - [Using Envelopes](envelope-usage.md): How middleware and backends read and change envelopes, with protocol-specific examples.
- [Commands: Protocol-Agnostic Operations](commands.md): The Command type that lets endpoints and backends exchange protocol-agnostic operations.
  - [Producing and Consuming Commands](commands-producers-consumers.md): How endpoints produce commands, how backends consume them, and the command parameter and builder APIs.
- [Extensions](extensions.md): The extension system for registering custom services, middleware, and plugins.
  - [Extension Types](extensions-types.md): Reference for each kind of extension Harmony supports.
  - [Implementing Extensions](extensions-implementing.md): How to implement custom services and middleware as extensions.
- [Config Hot-Reload Strategy](config-reload.md): How Harmony classifies configuration changes and applies them without a full restart.
- [DICOM Streaming Architecture](dicom-streaming.md): How Harmony streams large DICOM studies through a pipeline one instance at a time.
  - [DICOM Streaming Services](dicom-streaming-services.md): The endpoint and backend services that take part in DICOM streaming.
  - [DICOM Streaming Use Cases and Performance](dicom-streaming-use-cases.md): Example DICOM streaming pipelines and their performance characteristics.
  - [DICOM Streaming Errors, Testing, and Troubleshooting](dicom-streaming-troubleshooting.md): How DICOM streaming handles errors, how it is tested, and how to diagnose problems.

Last architecture review: not yet recorded. These files were moved into this section on 2026-10-24 without a content review.
