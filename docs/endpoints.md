# Endpoints

Endpoints provide an entry point into the router for specific kinds of requests. Endpoints may convert requests into standardised data formats such as JSON, to be passed to other parts of the application.

## Endpoint Types

### HTTP (Passthru)

A basic HTTP endpoint. 
- Accepts an HTTP request and converts it into an Envelope.
- Takes an Envelope and converts it into an HTTP response

```
[endpoints.<name>]
type = "http"
options = { path_prefix = "/<some-path>" } 
```

### FHIR

Extends the HTTP endpoint for FHIR
- Accepts an HTTP request and converts it into an Envelope.
- Takes an Envelope and converts it into an HTTP response

```
[endpoints.<name>]
type = "fhir"
options = { path_prefix = "/<some-path>" } 
```

### JMIX

JMIX endpoint registers a strict, fixed set of routes. Only the following are handled (under the configured path_prefix):
- GET {prefix}/api/jmix/{id}
- GET {prefix}/api/jmix/{id}/manifest
- GET {prefix}/api/jmix?studyInstanceUid=...
- POST {prefix}/api/jmix

```
[endpoints.<name>]
 type = "jmix"
 options = { path_prefix = "/<some-path>" }
```

See [jmix-dev-testing.md](../dev/jmix-dev-testing.md) for development testing.

### DICOMweb

Provides DICOMweb QIDO-RS and WADO-RS endpoints
- Accepts DICOMweb requests and converts them into Envelopes
- Currently returns 501 Not Implemented for all endpoints (skeleton implementation)
- Supports QIDO-RS queries (studies, series, instances) and WADO-RS retrieval (metadata, instances, frames, bulk data)

```
[endpoints.<name>]
service = "dicomweb"
[endpoints.<name>.options]
path_prefix = "/dicomweb"
```

Supported routes:
- `GET /dicomweb/studies` - Query for studies
- `GET /dicomweb/studies/{study_uid}/series` - Query for series
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances` - Query for instances
- `GET /dicomweb/studies/{study_uid}/metadata` - Retrieve study metadata
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/metadata` - Retrieve series metadata
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/metadata` - Retrieve instance metadata
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}` - Retrieve instance
- `GET /dicomweb/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/frames/{frame_numbers}` - Retrieve frames
- `GET /dicomweb/bulkdata/{bulk_data_uri}` - Bulk data retrieval
