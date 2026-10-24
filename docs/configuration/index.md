# Configuration

Reference for Harmony's configuration files, pipeline components, and validation schemas.

- [Configuration](configuration-file.md): Reference for the main configuration file, including networks, authentications, reload, and Runbeam Cloud.
  - [Environment Variable Substitution](environment-substitution.md): How to reference environment variables inside configuration files.
  - [Network Configuration](networks.md): How to configure networks, bind addresses, TLS, and WireGuard.
  - [Hot Configuration Reload](hot-reload.md): Which configuration changes reload without a restart, and how to use reload.
  - [Environment Variables](environment.md): Environment variables that Harmony reads at startup.
- [Configuration Schemas](schema.md): The harmony-dsl schemas that validate mesh, config, pipeline, and remote-ingress files.
- [Providers](providers.md): How providers resolve and synchronise resources from local files or Runbeam Cloud.
- [Resource References](resource-references.md): Syntax and resolution rules for referencing resources across providers.
- [Data Meshes](mesh.md): How to configure a data mesh, including ingress, egress, and JWT authentication between proxies.
- [Endpoints](endpoints.md): Endpoint types (HTTP, FHIR, JMIX, DICOMweb, DIMSE) and their connection settings.
- [Backends](backends.md): Backend types, connection settings, upstream headers, and target selection.
  - [HTTP Backends](backends-http.md): Configuration for the HTTP, FHIR, DICOMweb, and HTTP/3 backends.
  - [DICOM, Storage, and Echo Backends](backends-dicom-storage.md): Configuration for the DICOM SCU, storage, and echo backends.
- [Middleware](middleware.md): Reference for authentication, transform, path filter, and other built-in middleware.
  - [Transform Middleware](middleware-transforms.md): Reference for middleware that transforms or converts request and response data.
  - [Utility Middleware](middleware-utilities.md): Reference for the log dump, webhook, and passthru middleware.
- [Pipeline Middleware Split Configuration](middleware-split-configuration.md): How to configure pipeline middleware as a single list or as separate request and response lists.
- [Policies Middleware](policies-middleware.md): How to configure the policies middleware, its evaluation logic, and rule weighting.
  - [Policy Rules: Source and Access](policies-rules-source.md): Policy rule types that match on client address, path, location, headers, and rate.
  - [Policy Rules: Request Attributes](policies-rules-request.md): Policy rule types that match on time, method, user agent, content type, and query parameters.
  - [Policy Patterns and Troubleshooting](policies-patterns.md): Common policy patterns, best practices, troubleshooting, and examples.
- [Policy Rules Quick Reference](policy-rules-reference.md): Quick reference for every policy rule type and its options.
  - [Policy Rules Reference: Request Attributes](policy-rules-reference-request.md): Quick reference for the time, method, user agent, content type, and query parameter rules.
  - [Policy Rules Examples](policy-rules-examples.md): Complete policy configurations that combine several rule types.
- [Multi-Content-Type Support](content-types.md): How Harmony parses JSON, XML, CSV, form data, multipart, and binary bodies.
  - [Supported Content Types](content-types-formats.md): How each supported content type is parsed into normalised data.
  - [Content Type Security and Error Handling](content-types-handling.md): Security limits, error handling, fallback behaviour, and troubleshooting for content parsing.

The JOLT transform syntax is documented with its crate in [crates/transform/README.md](../../crates/transform/README.md).
