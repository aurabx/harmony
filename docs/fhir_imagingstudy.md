# FHIR ImagingStudy Pipeline Documentation

This document describes how to implement FHIR ImagingStudy endpoints that query DICOM backends using harmony-proxy, specifically for the endpoint:

```
GET [base]/ImagingStudy?patient=[patient-id]
```

## Overview

The pipeline converts REST FHIR requests to DICOM C-FIND operations and transforms DICOM study-level responses into FHIR ImagingStudy resources.

### Data Flow

```
HTTP Request → FHIR Endpoint → imagingstudy_filter → json_extractor → fhir_dimse_meta → [Transform] → DICOM Backend
                                                                                                              ↓
HTTP Response ← FHIR Bundle ← [Transform] ← json_extractor ← DICOM JSON Response
```

1. **HTTP Request**: `GET /ImagingStudy?patient=PID123`
2. **FHIR Endpoint**: Processes the REST request and extracts query parameters
3. **imagingstudy_filter**: Validates path matches "/ImagingStudy" pattern, rejects others with 404
4. **json_extractor**: Copies HTTP request body to `normalized_data`
5. **fhir_dimse_meta**: Sets `dimse_op="find"` in request metadata using metadata_set_dimse_op.json
6. **Transform Middleware** (optional): Maps query parameters for DICOM backend
7. **DICOM Backend**: Executes C-FIND STUDY with PatientID filter (uses dimse_op from metadata)
8. **json_extractor**: Processes DICOM JSON response
9. **Transform Middleware**: Converts DICOM study data to FHIR ImagingStudy Bundle
10. **HTTP Response**: Returns FHIR Bundle with ImagingStudy resources

## Implementation Approaches

### Approach A: Using JOLT Transform (Recommended)

Uses the existing transform middleware with a JOLT specification to convert DICOM JSON to FHIR.

**Pros:**
- No code changes required
- Declarative mapping specification
- Easy to modify field mappings
- Reusable transform patterns

**Cons:**
- JOLT syntax learning curve
- Limited to JSON transformations

### Approach B: Custom Middleware

Implements a dedicated Rust middleware for FHIR ImagingStudy conversion.

**Pros:**
- Full programmatic control
- Better error handling
- Performance optimizations
- Type safety

**Cons:**
- Requires Rust development
- More complex to maintain

## DICOM to FHIR Field Mapping

| DICOM Tag | DICOM Field | FHIR Field | Notes |
|-----------|-------------|------------|-------|
| `0020000D` | StudyInstanceUID | `id`, `identifier[0].value` | Primary identifier |
| `00100020` | PatientID | `subject.reference` | `Patient/{PatientID}` |
| `00100010` | PatientName | `subject.display` | Optional display name |
| `00080020` | StudyDate | `started` (date part) | Combined with StudyTime |
| `00080030` | StudyTime | `started` (time part) | Format: `YYYYMMDDTHHMM` |
| `00081030` | StudyDescription | `description` | Study description |
| `00200010` | StudyID | `identifier[1].value` | Secondary identifier |
| `00080060` | Modality* | `modality[].code` | Series-level, aggregated |

*Note: Modality is typically a series-level attribute but can be aggregated at study level.

## Sample DICOM Response Format

The DICOM backend returns JSON in this structure:

```json
{
  "operation": "find",
  "success": true,
  "matches": [
    {
      "0020000D": {"vr": "UI", "Value": ["1.2.826.0.1.3680043.9.7133.3280065491876470"]},
      "00100020": {"vr": "LO", "Value": ["PID156695"]},
      "00100010": {"vr": "PN", "Value": [{"Alphabetic": "Doe^John"}]},
      "00080020": {"vr": "DA", "Value": ["20241015"]},
      "00080030": {"vr": "TM", "Value": ["120000"]},
      "00081030": {"vr": "LO", "Value": ["Mock CT Study"]},
      "00200010": {"vr": "SH", "Value": ["1"]}
    }
  ]
}
```

## Target FHIR ImagingStudy Format

The pipeline should produce FHIR R4 Bundle responses:

```json
{
  "resourceType": "Bundle",
  "type": "searchset",
  "total": 1,
  "entry": [
    {
      "resource": {
        "resourceType": "ImagingStudy",
        "id": "1.2.826.0.1.3680043.9.7133.3280065491876470",
        "identifier": [
          {
            "system": "urn:dicom:uid",
            "value": "1.2.826.0.1.3680043.9.7133.3280065491876470"
          },
          {
            "system": "http://example.org/studyid",
            "value": "1"
          }
        ],
        "status": "available",
        "subject": {
          "reference": "Patient/PID156695",
          "display": "Doe^John"
        },
        "started": "2024-10-15T12:00:00Z",
        "description": "Mock CT Study"
      }
    }
  ]
}
```

## Configuration Examples

### Approach A: JOLT Transform Pipeline

Create `examples/config/pipelines/fhir_imagingstudy.toml`:

```toml
[pipelines.imagingstudy_query]
description = "GET /ImagingStudy?patient={id}"
networks = ["default"]
endpoints = ["fhir_imagingstudy_ep"]
middleware = [
    "imagingstudy_filter",
    "json_extractor",
    "fhir_dimse_meta",
    "dicom_to_fhir_transform"
]
backends = ["dicom_backend"]

[middleware.imagingstudy_filter]
type = "path_filter"
[middleware.imagingstudy_filter.options]
rules = ["/ImagingStudy"]

[middleware.fhir_dimse_meta]
type = "metadata_transform"
[middleware.fhir_dimse_meta.options]
spec_path = "examples/fhir_dicom/transforms/metadata_set_dimse_op.json"
apply = "left"
fail_on_error = true

[endpoints.fhir_imagingstudy_ep]
service = "http"
[endpoints.fhir_imagingstudy_ep.options]
path_prefix = "/ImagingStudy"

[middleware.dicom_to_fhir_transform]
type = "transform"
[middleware.dicom_to_fhir_transform.options]
spec_path = "examples/config/transforms/dicom_to_imagingstudy.json"
apply = "right"  # Transform backend response
fail_on_error = true

[backends.dicom_backend]
service = "dicom"
[backends.dicom_backend.options]
local_aet = "HARMONY_SCU"
aet = "PACSCLINIC"
host = "127.0.0.1"
port = 104
```

### Approach B: Custom Middleware Pipeline

Create `examples/config/pipelines/fhir_imagingstudy_custom.toml`:

```toml
[pipelines.imagingstudy_query]
description = "GET /ImagingStudy?patient={id} via custom middleware"
networks = ["default"]
endpoints = ["fhir_imagingstudy_ep"]
middleware = [
    "imagingstudy_filter",
    "json_extractor",
    "fhir_dimse_meta",
    "fhir_imagingstudy"
]
backends = ["dicom_backend"]

[endpoints.fhir_imagingstudy_ep]
service = "http"
[endpoints.fhir_imagingstudy_ep.options]
path_prefix = "/ImagingStudy"

[middleware.fhir_imagingstudy]
type = "fhir_imagingstudy"

[backends.dicom_backend]
service = "dicom"
[backends.dicom_backend.options]
local_aet = "HARMONY_SCU"
aet = "PACSCLINIC"
host = "127.0.0.1"
port = 104
```

## Testing with Mock Backend

For development and testing, you can use the mock DICOM backend:

```toml
[backends.dicom_backend]
service = "mock_dicom"
```

This provides realistic test data without requiring a real DICOM server.

## Authentication

To add authentication, include auth middleware in the pipeline:

```toml
middleware = [
    "middleware.jwt_auth",  # or basic_auth
    "middleware.json_extractor",
    "middleware.dicom_to_fhir_transform"
]

[middleware.jwt_auth]
# JWT configuration here
```

## Testing

### Manual Testing

Start harmony-proxy with your configuration:

```bash
./harmony-proxy --config examples/config/pipelines/fhir_imagingstudy.toml
```

Test the endpoint:

```bash
curl -s "http://localhost:8081/ImagingStudy?patient=PID156695" \
  -H "Accept: application/fhir+json" | jq .
```

### Expected Response

```json
{
  "resourceType": "Bundle",
  "type": "searchset",
  "total": 1,
  "entry": [
    {
      "resource": {
        "resourceType": "ImagingStudy",
        "id": "1.2.826.0.1.3680043.9.7133.3280065491876470",
        "identifier": [
          {
            "system": "urn:dicom:uid",
            "value": "1.2.826.0.1.3680043.9.7133.3280065491876470"
          }
        ],
        "status": "available",
        "subject": {
          "reference": "Patient/PID156695"
        },
        "started": "2024-10-15T12:00:00Z",
        "description": "Mock CT Study"
      }
    }
  ]
}
```

## Error Handling

The pipeline handles several error conditions:

1. **Patient Not Found**: Returns empty Bundle with `total: 0`
2. **DICOM Connection Error**: Returns HTTP 500 with FHIR OperationOutcome
3. **Invalid Query Parameters**: Returns HTTP 400 with FHIR OperationOutcome
4. **Transform Errors**: Returns HTTP 500 (if `fail_on_error: true`)

## Performance Considerations

- The DICOM C-FIND query operates at STUDY level for optimal performance
- Results are streamed and processed incrementally
- Consider implementing caching for frequently accessed studies
- Monitor DICOM connection pool usage for high-traffic scenarios

## Next Steps

1. **Series and Instance Levels**: Extend to support series and instance queries
2. **Additional FHIR Resources**: Implement Patient, Organization resources
3. **Advanced Filtering**: Support additional FHIR search parameters
4. **Bulk Operations**: Implement FHIR bulk data export
5. **Subscription**: Add FHIR subscription support for real-time updates