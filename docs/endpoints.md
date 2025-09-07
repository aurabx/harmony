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

Extends the HTTP endpoint for JMIX
- Defines standard JMIX API paths

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