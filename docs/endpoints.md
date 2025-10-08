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

### DICOMweb

Extends the HTTP endpoint for DICOMweb
- Accepts a DICOMweb request and converts it into an Envelope.
- Takes an Envelope and converts it into a DICOMweb response

```
[endpoints.<name>]
type = "dicomweb"
options = { path_prefix = "/<some-path>" } 
```