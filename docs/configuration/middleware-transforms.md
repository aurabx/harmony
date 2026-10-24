---
title: Transform Middleware
description: Reference for middleware that transforms or converts request and response data.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/middleware.md
---

# Transform Middleware

Reference for middleware that transforms or converts request and response data. This page is part of [Middleware](middleware.md).

## Transformation

### Transform (JOLT)
Applies JSON-to-JSON transformations using JOLT specifications. Supports configurable application on request/response sides with error handling options.

The transform works on JSON bodies only. A response with a text body, such as `text/html`, `text/plain` or `application/xml`, passes through unchanged. So does a body that is not valid JSON, even when its `Content-Type` says JSON. Do not rely on a response `transform` to remove data from a non-JSON response.

**Cloud Integration**: When configurations are sourced from Runbeam Cloud, transform specifications (JOLT files) are automatically downloaded before the configuration is applied. The gateway:
- Extracts transform IDs from middleware `spec_path` fields
- Downloads JOLT specifications from Runbeam Cloud Transform API
- Writes transform files to the configured `transforms_path` directory
- Overwrites existing transforms with the same ID
- Fails the config change if any transform download fails

This ensures that all referenced transform specifications are available before pipeline execution begins. For manual configurations, transform files must be provided separately in the transforms directory.

## Metadata Transform

Applies JOLT transformations to request metadata (the HashMap&lt;String, String&gt; in RequestDetails). This allows dynamic modification of metadata fields that control backend behavior.

Config keys:
- `spec_path` (string, required): Path to JOLT specification file
- `apply` (string, optional): When to apply - "left", "right", or "both" (default: "left")
- `fail_on_error` (bool, optional): Whether to fail request on transform errors (default: true)

Example:
```toml
[middleware.fhir_dimse_meta]
type = "metadata_transform"
[middleware.fhir_dimse_meta.options]
spec_path = "transforms/metadata_set_command.json"
apply = "left"
fail_on_error = true
```

Behavior:
- Converts metadata to JSON object for JOLT processing
- Only string-valued outputs from JOLT are written back to metadata
- Preserves existing metadata fields not modified by transform
- Common use case: setting the `Command` on the envelope to control backend operations. If the transform output includes a `command` field, it is deserialized back as a typed `Command`. See [commands.md](../architecture/commands.md).

## JMIX Builder

Builds JMIX envelopes from DICOM operation responses. Handles caching, indexing, and ZIP file creation for JMIX packages.

Config:
```toml
[middleware.jmix_builder]
type = "jmix_builder"
[middleware.jmix_builder.options]
# Performance optimization flags
skip_hashing = true   # Skip SHA256 hashing for faster processing
skip_listing = true   # Skip DICOM files from files.json manifest
```

**Configuration options:**
- `skip_hashing` (bool, optional, default: false): Skip SHA256 file hashing for faster processing
- `skip_listing` (bool, optional, default: false): Skip DICOM files from files.json manifest

**Left side behavior (request processing):**
- Processes GET/HEAD requests for JMIX endpoints (`/api/jmix/{id}`, `/api/jmix?studyInstanceUid=...`)
- Serves cached JMIX packages if they exist locally
- Returns manifest.json for manifest requests (`/api/jmix/{id}/manifest`)
- Sets `skip_backends=true` and response metadata when serving from cache
- Passes through to backends when no local package exists

**Right side behavior (response processing):**
- Detects DICOM "move"/"get" responses containing `folder_path` and `instances`
- Creates JMIX packages under storage using jmix-rs builder
- Copies DICOM files from the backend folder into the package payload
- Writes manifest.json and metadata.json files
- Creates ZIP files for distribution
- Indexes packages by StudyInstanceUID for query lookup
- Cleans up temporary DICOM files after successful ZIP creation

This middleware is typically used with JMIX endpoints that bridge to DICOM backends, automatically converting DICOM responses into distributable JMIX packages.

## DICOMweb Bridge

Bridges DICOMweb HTTP requests (QIDO-RS/WADO-RS) to DICOM operations and converts responses back to DICOMweb format.

Config:
```toml
[middleware.dicomweb_bridge]
type = "dicomweb_bridge"
```

**Left side behavior (DICOMweb → DICOM):**
- Maps DICOMweb URLs to DICOM operations:
  - `/studies` → C-FIND at study level
  - `/studies/{study}/series` → C-FIND at series level  
  - `/studies/{study}/series/{series}/instances` → C-FIND at instance level
  - `/studies/{study}/series/{series}/instances/{instance}` → C-GET (WADO) or C-FIND (QIDO)
  - `/studies/.../metadata` → C-FIND with full metadata
  - `/studies/.../frames/{frames}` → C-GET for frame extraction
- Sets `envelope.command` with the appropriate `Operation` (e.g., `Query` for QIDO, `Retrieve` for WADO). See [commands.md](../architecture/commands.md).
- Converts query parameters to DICOM identifiers with hex tags
- Processes `includefield` parameter for attribute filtering
- Sets appropriate return keys based on query level and includefield
- Distinguishes between QIDO (JSON) and WADO (binary) based on Accept headers

**Right side behavior (DICOM → DICOMweb):**
- **QIDO responses**: Converts DICOM find results to DICOMweb JSON arrays
- **WADO metadata**: Returns filtered JSON metadata based on includefield
- **WADO instances**: Creates multipart/related responses with DICOM files
- **WADO frames**: Decodes DICOM pixel data to JPEG/PNG images
- Handles both single-frame and multi-frame responses
- Supports content negotiation (Accept: image/jpeg, image/png)
- Provides proper error responses for unsupported transfer syntaxes

**Features:**
- Full DICOMweb QIDO-RS and WADO-RS compliance
- Automatic DICOM tag name to hex conversion using dicom-rs StandardDataDictionary
- Support for multiple query parameter values
- Includefield filtering for bandwidth optimization
- Multipart response handling for bulk data
- Frame-level image extraction with format conversion

**Command integration**: The DICOM→DICOMweb middleware (reverse direction) reads the Command via `envelope.resolve_operation()` on the `ResponseEnvelope` to dispatch the correct response formatting.

This middleware enables DICOMweb endpoints to communicate with traditional DICOM PACS systems via DIMSE protocols.

## DICOM Flatten

Flattens DICOM JSON structures for simplified processing, converting between standard DICOM JSON format (Part 18) and flat key-value pairs. Useful for downstream systems that require simplified DICOM data representation.

### Configuration

Config keys:
- `apply` (string, optional): When to apply - "left" (flatten requests), "right" (unflatten responses), or "both" (default)

### Example

```toml
[middleware.dicom_flatten_example]
type = "dicom_flatten"
[middleware.dicom_flatten_example.options]
apply = "both"  # Flatten on request, unflatten on response
```

### Behavior

**Flatten (left side):**
- Converts standard DICOM JSON with `{vr, Value}` structure to simple key-value pairs
- Tag ID → scalar value or nested structure for sequences
- Person Name (PN) VR: Extracts "Alphabetic" field
- Preserves VR metadata internally for reconstruction
- Example: `{"00100020": {"vr": "LO", "Value": ["PID123"]}}` → `{"00100020": "PID123"}`

**Unflatten (right side):**
- Reconstructs standard DICOM JSON from flattened form
- Uses preserved VR metadata to restore proper structure
- Recreates sequences (SQ) from nested arrays
- Restores Person Name structure with Alphabetic field
- Example: `{"00100020": "PID123"}` → `{"00100020": {"vr": "LO", "Value": ["PID123"]}}`

**Supported VR types:**
- Scalars: LO, UI, SH, DA, TM, DT, PN (with special handling)
- Sequences (SQ): Recursive flattening of nested items
- Multi-valued: Arrays preserved as-is
- Empty values: Handled as return keys

**Snapshot preservation:**
- Original data snapshot stored before transformation
- Enables inspection of pre-transformation state
- Compatible with debugging and log dump middleware

### Use Cases

- Simplify DICOM data for frontend/API consumption
- Bridge DICOM backends with systems expecting flat structures
- Round-trip transformations: flatten for processing, unflatten for DICOM compliance
- DICOM data export to simplified formats

## JSON Extractor

Ensures the request body is parsed as JSON into `normalized_data`. This is useful when subsequent middleware expects structured JSON data but the content type might not have triggered automatic parsing, or to guarantee `normalized_data` is populated.

Config:
```toml
[middleware.ensure_json]
type = "json_extractor"
# No options required
```
