# Configuration

Reference for Harmony's configuration files, pipeline components, and validation schemas.

- [Configuration](configuration-file.md): Reference for the main configuration file, including networks, authentications, reload, and Runbeam Cloud.
- [Configuration Schemas](schema.md): The harmony-dsl schemas that validate mesh, config, pipeline, and remote-ingress files.
- [Providers](providers.md): How providers resolve and synchronise resources from local files or Runbeam Cloud.
- [Resource References](resource-references.md): Syntax and resolution rules for referencing resources across providers.
- [Data Meshes](mesh.md): How to configure a data mesh, including ingress, egress, and JWT authentication between proxies.
- [Endpoints](endpoints.md): Endpoint types (HTTP, FHIR, JMIX, DICOMweb, DIMSE) and their connection settings.
- [Backends](backends.md): Backend types, connection settings, upstream headers, and target selection.
- [Middleware](middleware.md): Reference for authentication, transform, path filter, and other built-in middleware.
- [Pipeline Middleware Split Configuration](middleware-split-configuration.md): How to configure pipeline middleware as a single list or as separate request and response lists.
- [Policies Middleware](policies-middleware.md): How to configure the policies middleware, its evaluation logic, and rule weighting.
- [Policy Rules Quick Reference](policy-rules-reference.md): Quick reference for every policy rule type and its options.
- [Multi-Content-Type Support](content-types.md): How Harmony parses JSON, XML, CSV, form data, multipart, and binary bodies.

The JOLT transform syntax is documented with its crate in [crates/transform/README.md](../../crates/transform/README.md).
