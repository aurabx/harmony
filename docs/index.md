---
okf_version: "0.2"
---

# Harmony Proxy Documentation

Harmony is a Rust proxy and gateway for data meshes. It receives data over HTTP, FHIR, JMIX, DICOMweb, and DICOM DIMSE. It runs that data through configurable pipelines of middleware and sends it on to backends. Healthcare integration is a primary use. Operators use it to connect systems securely, and developers extend it with new protocols and middleware.

## Sections

1. [Context](context/index.md): Product vision, domain terms, and stakeholders.
2. [Architecture](architecture/index.md): How Harmony works today, from protocol adapters through pipelines to backends.
3. [Architecture Decision Records](adr/index.md): The log of architectural decisions.
4. [Plans](plans/index.md): Roadmap, epics, and feature specs for future work.
5. [API](api/index.md): The management API.
6. [Configuration](configuration/index.md): Configuration files, endpoints, backends, middleware, and schemas.
7. [Integrations](integrations/index.md): DICOM DIMSE and FHIR ImagingStudy integration.
8. [Security](security/index.md): Authentication, authorisation, and key management.
9. [Guides](guides/index.md): Getting started, testing, and deployment.
10. [Operations](operations/index.md): Monitoring and runbooks for production.
11. [Migrations](migrations/index.md): Data and configuration migration history.
12. [Changelog](changelog/index.md): Release history.

## Quick reference

- **Run locally:** [guides/getting-started.md](guides/getting-started.md)
- **Run the tests:** [guides/testing.md](guides/testing.md)
- **Deploy:** [guides/deployment.md](guides/deployment.md)
- **Write a configuration file:** [configuration/configuration-file.md](configuration/configuration-file.md)
- **Understand the request flow:** [architecture/router.md](architecture/router.md)
- **Example configuration:** [examples/dicom-streaming](../examples/dicom-streaming/README.md)

## Conventions

- Put temporary files in `./tmp` inside the working directory, not in the system `/tmp`.
- Never commit secrets. Load them from environment variables or a secret manager. See [security/overview.md](security/overview.md).
- Use relative paths for every cross-reference.

## Related files

- [README](../readme.md)
- [CLAUDE.md](../CLAUDE.md) and [AGENTS.md](../AGENTS.md)
- [DEVELOPER.md](../DEVELOPER.md) and [CONTRIBUTING.md](../CONTRIBUTING.md)
