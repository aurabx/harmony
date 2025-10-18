# Endpoints

**Last Updated**: 2025-01-18 (Phase 6)

Endpoints define protocol entry points into the Harmony pipeline. Each endpoint is associated with a **Service** that handles protocol-specific data conversion to/from the internal `Envelope` format.

## Architecture

Endpoints work with **Protocol Adapters** (see [adapters.md](adapters.md)) to process requests:

```
Protocol Request → Protocol Adapter → Service (via Endpoint config) → RequestEnvelope → Pipeline
```

The endpoint configuration specifies:
- Which service type to use (e.g., `http`, `fhir`, `jmix`, `dicomweb`, `dicom`)
- Service-specific options (e.g., `path_prefix` for HTTP endpoints, `local_aet` for DICOM)
- How the service should construct request/response envelopes

## Endpoint Types

### HTTP (Passthru)

A basic HTTP endpoint handled by `HttpAdapter`.

**Service behavior**:
- Accepts an HTTP request and converts it into a `RequestEnvelope`
- Takes a `ResponseEnvelope` and converts it into an HTTP response
- Preserves headers, query parameters, and body

**Configuration**:
```toml
[endpoints.<name>]
service = "http"
[endpoints.<name>.options]
path_prefix = "/<some-path>"
```

**Example**: HTTP passthrough to backend
```toml
[endpoints.api_passthrough]
service = "http"
[endpoints.api_passthrough.options]
path_prefix = "/api"
```

### FHIR

Extends the HTTP endpoint with FHIR-specific handling.

**Service behavior**:
- Accepts an HTTP request and converts it into a `RequestEnvelope`
- Provides FHIR-aware request/response handling
- Takes a `ResponseEnvelope` and converts it into a FHIR-compliant HTTP response

**Configuration**:
```toml
[endpoints.<name>]
service = "fhir"
[endpoints.<name>.options]
path_prefix = "/<some-path>"
```

**Example**: FHIR resource server
```toml
[endpoints.fhir_server]
service = "fhir"
[endpoints.fhir_server.options]
path_prefix = "/fhir"
```

### JMIX

JMIX endpoint registers a strict, fixed set of routes for the JMIX healthcare data exchange format.

**Service behavior**:
- Registers fixed routes under the configured `path_prefix`
- Handles JMIX data package operations

**Supported routes** (under configured `path_prefix`):
- `GET {prefix}/api/jmix/{id}` - Retrieve JMIX package
- `GET {prefix}/api/jmix/{id}/manifest` - Retrieve package manifest
- `GET {prefix}/api/jmix?studyInstanceUid=...` - Query by Study Instance UID
- `POST {prefix}/api/jmix` - Create JMIX package

**Configuration**:
```toml
[endpoints.<name>]
service = "jmix"
[endpoints.<name>.options]
path_prefix = "/<some-path>"
```

**Example**: JMIX data exchange
```toml
[endpoints.jmix_exchange]
service = "jmix"
[endpoints.jmix_exchange.options]
path_prefix = "/data"
```

See [jmix-dev-testing.md](../dev/jmix-dev-testing.md) for development testing.

### DICOMweb

Provides DICOMweb QIDO-RS (Query) and WADO-RS (Retrieve) endpoints.

**Service behavior**:
- Accepts DICOMweb requests and converts them into `RequestEnvelope`
- Supports QIDO-RS queries and WADO-RS retrieval operations
- Currently returns 501 Not Implemented for all endpoints (skeleton implementation)

**Configuration**:
```toml
[endpoints.<name>]
service = "dicomweb"
[endpoints.<name>.options]
path_prefix = "/dicomweb"
```

**Supported routes**:
- `GET /dicomweb/studies` - Query for studies (QIDO-RS)
- `GET /dicomweb/studies/{study_uid}/series` - Query for series (QIDO-RS)
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances` - Query for instances (QIDO-RS)
- `GET /dicomweb/studies/{study_uid}/metadata` - Retrieve study metadata (WADO-RS)
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/metadata` - Retrieve series metadata (WADO-RS)
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/metadata` - Retrieve instance metadata (WADO-RS)
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}` - Retrieve instance (WADO-RS)
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/frames/{frame_numbers}` - Retrieve frames (WADO-RS)
- `GET /dicomweb/bulkdata/{bulk_data_uri}` - Bulk data retrieval (WADO-RS)

**Example**: DICOMweb PACS interface
```toml
[endpoints.dicomweb_pacs]
service = "dicomweb"
[endpoints.dicomweb_pacs.options]
path_prefix = "/pacs"
```
