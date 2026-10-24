# Architecture

How Harmony works today. This section describes the current system, not planned changes. Planned work lives in [plans](../plans/index.md).

- [System Description](overview.md): High-level overview of Harmony's components and how they fit into the Runbeam architecture.
- [Architecture Diagrams](diagrams.md): Diagrams of the unified request flow, data flow, and orchestration flow.
- [Router and Pipeline Architecture](router.md): How requests move from a protocol adapter through the PipelineExecutor and back.
- [Protocol Adapters](adapters.md): The ProtocolAdapter trait, the built-in HTTP, HTTP/3 and DIMSE adapters, and how to add a new one.
- [Envelope: Data Exchange Format](envelope.md): The RequestEnvelope and ResponseEnvelope types that carry data through every pipeline.
- [Commands: Protocol-Agnostic Operations](commands.md): The Command type that lets endpoints and backends exchange protocol-agnostic operations.
- [Extensions](extensions.md): The extension system for registering custom services, middleware, and plugins.
- [Config Hot-Reload Strategy](config-reload.md): How Harmony classifies configuration changes and applies them without a full restart.
- [DICOM Streaming Architecture](dicom-streaming.md): How Harmony streams large DICOM studies through a pipeline one instance at a time.

Last architecture review: not yet recorded. These files were moved into this section on 2026-10-24 without a content review.
