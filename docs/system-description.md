# System Description

**Last Updated**: 2025-01-18 (Phase 6)

Harmony is a proxy/gateway designed to handle, transform, and route data between different systems using a **protocol-agnostic architecture**.

## Key Components

- **[Protocol Adapters](adapters.md)**: Handle protocol-specific I/O (HTTP, DIMSE, HL7, etc.)
- **[Pipeline](router.md)**: Unified execution engine for all protocols
- **[Endpoints](endpoints.md)**: Protocol entry points (HTTP paths, DICOM AE titles, etc.)
- **[Middleware](middleware.md)**: Authentication, transformation, and enrichment
- **[Backends](backends.md)**: Communication with external systems
- **[Envelopes](envelope.md)**: Protocol-agnostic data exchange format

## Architecture

```
Protocol Request (HTTP/DIMSE/HL7/...)
  ↓
Protocol Adapter (HttpAdapter/DimseAdapter/...)
  ↓ Converts to
RequestEnvelope (protocol-agnostic)
  ↓
PipelineExecutor (unified business logic)
  ├─ Endpoint Service
  ├─ Incoming Middleware
  ├─ Backend
  └─ Outgoing Middleware
  ↓
ResponseEnvelope
  ↓ Converts back
Protocol Adapter
  ↓
Protocol Response
```


### Runbeam Architecture

The Harmony proxy architecture defines how users, organisations, endpoints, and services interact within the Runbeam network. The model separates responsibilities into clear abstractions for routing, policy enforcement, and inter-organisation communication.

#### Users, Teams, and Orgs (these are NOT part of Harmony)

* **Users** belong to **Teams**.
* **Teams** are grouped into **Orgs**, which are the fundamental unit of organisation.
* **Orgs** (and sub-groups) organise endpoints. By default, all endpoints within an Org or Group can communicate.
* Policies can restrict communication further. Endpoints cannot be shared outside their Org, but Orgs can be members of multiple Groups.

#### Gateway and Endpoint

* A **Gateway** represents a Harmony entry point. It maps to an IP or DNS address (this project)
* An **Endpoint** attaches to a Gateway and registers with Runbeam. Endpoints have a globally unique URI in the Runbeam network, allowing routing without needing the underlying Gateway address.

#### Service

* **Services** are the backend applications or systems behind a Gateway.
* Each Service declaration specifies which backend services are exposed on an Endpoint.
* This tells Runbeam which services are available for routing requests.

#### Pipeline

* A **Pipeline** handles the flow of traffic between Endpoints and Services.
* Pipelines are designed for inter-organisation communication. They use **Network Endpoints**, which behave like regular Endpoints but are specialised for network-level traffic.
* Pipelines can be extended with:

    * **Transforms** – perform protocol or payload modifications.
    * **Policy + Rules** – enforce access control (e.g., “can Endpoint A talk to Endpoint B?”).

#### Network

* A **Network** is a higher-level abstraction built on Pipelines, designed for inter-organisation connectivity and routing.
* Networks allow secure, policy-controlled communication between different Orgs while leveraging the same Endpoint/Gateway structure.
