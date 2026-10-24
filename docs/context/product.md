---
title: Product
description: What Harmony is, who uses it, and what it is for.
status: active
updated: 2026-10-24
type: Context
related:
  - architecture/overview.md
  - context/domain.md
---

# Product

## What Harmony is

Harmony is a proxy and gateway for data meshes. It sits between systems that need to exchange data. It receives a request, runs it through a pipeline, and sends it on to a backend. It then returns the response to the caller.

Harmony speaks several protocols. It handles HTTP and JSON, FHIR, JMIX, DICOMweb, and DICOM DIMSE. It can convert between them, for example from DICOMweb to DIMSE. Healthcare integration is a primary use, but the pipeline model is not specific to healthcare.

Harmony is written in Rust. It is built by Aurabox and is part of the Runbeam platform. The project is under active development. The current version is in `Cargo.toml`.

## Who uses it

- **Platform teams** who build data meshes or integration hubs.
- **Developers** who connect HTTP services to healthcare systems such as PACS and FHIR servers.
- **Operators** who need request and response pipelines that they can configure and audit.

## What it does

- **Routes requests.** Pipelines connect endpoints to backends. See [Router and Pipeline Architecture](../architecture/router.md).
- **Runs middleware.** Middleware handles authentication, access policies, path filtering, and data transforms. See [Middleware](../configuration/middleware.md).
- **Converts protocols.** Protocol adapters turn HTTP and DIMSE traffic into a common envelope. See [Protocol Adapters](../architecture/adapters.md).
- **Reloads configuration.** Most configuration changes apply without a restart. See [Hot Configuration Reload](../configuration/hot-reload.md).
- **Joins a mesh.** Several Harmony instances can route requests to each other. See [Data Meshes](../configuration/mesh.md).
- **Connects to Runbeam Cloud.** An optional integration lets Runbeam Cloud manage the gateway's configuration. See [Runbeam Cloud Integration](../integrations/runbeam-cloud.md).

## Where Harmony fits in Runbeam

Runbeam is a platform for data meshes. Its users, teams, and organisations live in Runbeam Cloud, not in Harmony. Harmony is the gateway. Each Harmony instance is one entry point into the Runbeam network.

When cloud integration is on, Harmony authorises itself with Runbeam Cloud and polls it for configuration. When it is off, Harmony runs on its own from local configuration files. Cloud integration is off by default.
