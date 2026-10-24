---
title: Domain Glossary
description: Definitions of the terms used across Harmony's code, configuration, and documentation.
status: active
updated: 2026-10-24
type: Context
related:
  - context/product.md
  - configuration/configuration-file.md
---

# Domain Glossary

Terms are grouped by area. Where a term is also a configuration table, the table name is shown in code style.

## Pipeline model

- **Pipeline** (`[pipelines]`): Connects one or more endpoints to one or more backends, with ordered middleware between them. A pipeline is bound to one or more networks.
- **Endpoint** (`[endpoints]`): Where a request enters a pipeline. An HTTP path or a DICOM listener are both endpoints. See [Endpoints](../configuration/endpoints.md).
- **Backend** (`[backends]`): Where a pipeline sends a request. Examples are an upstream HTTP API, a PACS, or file storage. See [Backends](../configuration/backends.md).
- **Middleware** (`[middleware]`): A step that runs on the request before the backend, or on the response after it. Authentication, policies, and transforms are middleware. See [Middleware](../configuration/middleware.md).
- **Service** (`[services]`): The type of an endpoint or backend, such as `http`, `fhir`, `dicom_scp`, or `dicom_scu`. An endpoint or backend names its service with the `service` key.
- **Network** (`[network]`): A named listener with a bind address and port. Pipelines attach to networks. A network can also use WireGuard. See [Network Configuration](../configuration/networks.md).

## Connections and credentials

- **Target** (`[targets]`): Shared connection settings for backends, such as host, protocol, and authentication. A backend points to one with `target_ref`.
- **Peer** (`[peers]`): Shared connection settings for endpoints. An endpoint points to one with `peer_ref`.
- **Authentication** (`[authentications]`): A named credential, such as a bearer token. Targets and peers refer to it by its key.
- **Provider** (`[provider]`): Where Harmony loads resources from. A provider is either local files or a remote service such as Runbeam Cloud. See [Providers](../configuration/providers.md).

## Access control

- **Policy** (`[policies]`): A named group of rules. The `policies` middleware evaluates policies on each request.
- **Rule** (`[rules]`): One check inside a policy, such as an IP allow list or a rate limit. See [Policies Middleware](../configuration/policies-middleware.md).

## Mesh

- **Mesh** (`[mesh]`): A group of Harmony instances that route requests to each other. See [Data Meshes](../configuration/mesh.md).
- **Ingress** (`[ingress]`): How this instance accepts requests from other mesh members.
- **Egress** (`[egress]`): How this instance sends requests to other mesh members.
- **Remote ingress** (`[remote_ingress]`): A catalogue entry for another member's ingress.

## Internal model

- **Protocol adapter**: Code that turns protocol traffic (HTTP, HTTP/3, DIMSE) into an envelope and back. See [Protocol Adapters](../architecture/adapters.md).
- **Envelope**: The protocol-neutral form of a request (`RequestEnvelope`) or response (`ResponseEnvelope`) inside a pipeline. See [Envelope](../architecture/envelope.md).
- **ProtocolCtx**: Protocol details that travel with an envelope. It holds the protocol, the raw payload, metadata such as the calling AE title, attributes such as HTTP headers, and the typed command.
- **Command**: A protocol-neutral operation, such as query, retrieve, or store, that an endpoint sets and a backend acts on. See [Commands](../architecture/commands.md).
- **PipelineExecutor**: The component that runs a pipeline for one request.

## Healthcare protocols

- **FHIR**: HL7 Fast Healthcare Interoperability Resources. A REST and JSON standard for clinical data.
- **DICOM**: The standard for medical images and their metadata.
- **DIMSE**: DICOM's native network protocol. Its operations include C-ECHO, C-FIND, C-MOVE, C-GET, and C-STORE. See [DIMSE Integration](../integrations/dicom-dimse.md).
- **SCU and SCP**: Service Class User and Service Class Provider. An SCU starts a DIMSE request and an SCP answers it. In Harmony, `dicom_scu` is a backend and `dicom_scp` is an endpoint.
- **AE title (AET)**: The name of a DICOM application. It is 1 to 16 characters long.
- **PACS**: Picture Archiving and Communication System. A store for medical images.
- **DICOMweb**: DICOM over HTTP. QIDO-RS searches, WADO-RS retrieves, and STOW-RS stores.
- **JMIX**: A packaging format for exchanging medical imaging studies between systems.

## Runbeam

- **Runbeam Cloud**: The hosted service that can manage Harmony's configuration. See [Runbeam Cloud Integration](../integrations/runbeam-cloud.md).
- **Machine token**: The credential Harmony uses to call Runbeam Cloud after it has been authorised.
- **Gateway**: In Runbeam terms, one Harmony instance.
