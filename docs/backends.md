# Backends

Backends enable the router to talk to Targets. A backend can one or more targets (but must know how to select from them).
Backends turn Envelopes into requests to the backend, then convert the response back into an Envelope.

## Backend Types

### HTTP (Passthru)

A basic HTTP backend.
- Accepts an Envelope and converts it to an HTTP request
- Takes HTTP response and converts it to an Envelope

### FHIR

Extends the HTTP backend for FHIR

### DICOMweb

Extends the HTTP backend for DICOMweb

### DICOM

A basic DICOM backend.
- Takes an Envelope and converts it into a DICOM request
- Accepts a DICOM response and converts it to an Envelope

