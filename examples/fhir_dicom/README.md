# FHIR-DICOM Integration Example

This example demonstrates a complete FHIR-to-DICOM-to-FHIR pipeline using Harmony as a bridge between FHIR clients and DICOM PACS systems.

## Overview

The pipeline converts REST FHIR requests into DICOM C-FIND operations and transforms DICOM study-level responses back into FHIR ImagingStudy resources with JMIX download endpoints.

**Key Features:**
- **Dynamic Query Parameters**: Extract patient ID and other search parameters from FHIR requests
- **Context-Aware Transforms**: JOLT transforms can access request context (query params, headers, etc.)
- **JMIX Integration**: Automatic generation of JMIX API URLs for study downloads
- **Standards Compliant**: FHIR R4 ImagingStudy Bundle responses with proper endpoint references

## Quick Start

### 1. Start the Service

```bash
cd /Users/xtfer/working/runbeam/harmony-proxy
cargo run -- --config examples/fhir_dicom/config.toml
```

### 2. Query for Imaging Studies

```bash
# Search by patient ID
curl "http://localhost:8081/fhir/ImagingStudy?patient=PID156695" \
  -H "Accept: application/fhir+json" | jq .

# Search by study identifier
curl "http://localhost:8081/fhir/ImagingStudy?identifier=1.2.3.4.5" \
  -H "Accept: application/fhir+json" | jq .

# Multiple parameters
curl "http://localhost:8081/fhir/ImagingStudy?patient=PID123&modality=CT" \
  -H "Accept: application/fhir+json" | jq .
```

### 3. Management API

```bash
# System information
curl http://127.0.0.1:9091/admin/info | jq .

# List all pipelines
curl http://127.0.0.1:9091/admin/pipelines | jq .
```

## Pipeline Architecture

### Data Flow

```
┌─────────────────────────────────────────────────────────────┐
│ HTTP Request: GET /fhir/ImagingStudy?patient=PID123        │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
         ┌────────────────────────┐
         │ imagingstudy_filter    │  Validate path = /ImagingStudy
         └────────┬───────────────┘
                  │
                  ▼
         ┌────────────────────────┐
         │  query_to_target       │  Extract: patient → PatientID
         │  (metadata_transform)  │         identifier → StudyInstanceUID
         └────────┬───────────────┘         modality → Modality
                  │
                  ▼
         ┌────────────────────────┐
         │  json_extractor        │  Normalize request body
         └────────┬───────────────┘
                  │
                  ▼
         ┌────────────────────────┐
         │  fhir_dimse_meta       │  Set dimse_op="find"
         └────────┬───────────────┘
                  │
                  ▼
         ┌────────────────────────┐
         │ fhir_to_dicom_transform│  Build DICOM identifier from
         │ (with context)         │  context.target_details.metadata
         └────────┬───────────────┘
                  │
                  ▼
         ┌────────────────────────┐
         │   DICOM Backend        │  C-FIND STUDY
         │   (mock_dicom)         │  Query: PatientID=PID123
         └────────┬───────────────┘
                  │
                  ▼
         ┌────────────────────────┐
         │ enrich_jmix_urls       │  Add: _jmix_url for each study
         │ (with context)         │  /api/jmix?studyInstanceUid=xxx
         └────────┬───────────────┘
                  │
                  ▼
         ┌────────────────────────┐
         │ dicom_to_fhir_transform│  Convert to FHIR ImagingStudy
         │ (with context)         │  Include endpoint references
         └────────┬───────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────────────────┐
│ HTTP Response: FHIR Bundle                                  │
│ - ImagingStudy resources                                    │
│ - endpoint[].address = JMIX download URLs                   │
└─────────────────────────────────────────────────────────────┘
```

### Middleware Chain

**Left Side (Request Processing):**
1. `imagingstudy_filter` - Path validation (`path_filter`)
2. `query_to_target` - Query param extraction (`metadata_transform`)
3. `json_extractor` - Request normalization
4. `fhir_dimse_meta` - Set DIMSE operation (`metadata_transform`)
5. `fhir_to_dicom_transform` - Build DICOM query (`transform` with context)

**Right Side (Response Processing):**
6. `enrich_jmix_urls` - Add JMIX URLs (`transform` with context)
7. `dicom_to_fhir_transform` - Convert to FHIR (`transform` with context)

## Configuration

### Networks

- **Default Network**: `0.0.0.0:8081` - Client-facing FHIR endpoints
- **Management Network**: `127.0.0.1:9091` - Management API (local access only)

### Supported Query Parameters

The pipeline extracts and maps the following FHIR search parameters to DICOM tags:

| FHIR Parameter | DICOM Tag | Description |
|----------------|-----------|-------------|
| `patient` | `00100020` (PatientID) | Patient identifier |
| `identifier` | `0020000D` (StudyInstanceUID) | Study unique identifier |
| `modality` | `00080060` (Modality) | Study modality (CT, MR, etc.) |
| `studyDate` | `00080020` (StudyDate) | Date of study |
| `accessionNumber` | `00080050` (AccessionNumber) | Accession number |

### Backend Configuration

**Mock Backend (Development):**
```toml
[backends.dicom_backend]
service = "mock_dicom"
```

**Production DICOM PACS:**
```toml
[backends.dicom_backend]
service = "dicom"
[backends.dicom_backend.options]
local_aet = "HARMONY_SCU"
aet = "PACS_SCP"
host = "pacs.example.com"
port = 104
```

## JOLT Transforms

The pipeline uses four JOLT transform specifications:

### 1. `query_to_target_details.json`

Extracts FHIR query parameters and maps them to `target_details.metadata`:

```json
{
  "query_params": {
    "patient": ["PID123"]
  }
}
```
↓
```json
{
  "metadata": {
    "PatientID": "PID123"
  }
}
```

**Middleware:** `metadata_transform` with `transform_target = "target_details"`

### 2. `fhir_to_dicom_params.json`

Builds DICOM C-FIND identifier from context:

```json
{
  "context": {
    "target_details": {
      "metadata": { "PatientID": "PID123" }
    }
  }
}
```
↓
```json
{
  "data": {
    "dimse_identifier": {
      "00100020": { "vr": "LO", "Value": ["PID123"] }
    }
  }
}
```

**Key Feature:** Uses `context` to access query parameters without modifying them.

### 3. `enrich_with_jmix_urls.json`

Adds JMIX API URLs to each study:

```json
{
  "data": {
    "matches": [
      { "0020000D": { "Value": ["1.2.3.4.5"] } }
    ]
  }
}
```
↓
```json
{
  "data": {
    "matches": [
      {
        "0020000D": { "Value": ["1.2.3.4.5"] },
        "_jmix_url": "/api/jmix?studyInstanceUid=1.2.3.4.5"
      }
    ]
  }
}
```

### 4. `dicom_to_imagingstudy_simple.json`

Converts DICOM response to FHIR ImagingStudy Bundle with JMIX endpoints:

```json
{
  "resourceType": "Bundle",
  "type": "searchset",
  "entry": [
    {
      "resource": {
        "resourceType": "ImagingStudy",
        "id": "1.2.3.4.5",
        "status": "available",
        "subject": { "reference": "Patient/PID123" },
        "endpoint": [
          {
            "address": "/api/jmix?studyInstanceUid=1.2.3.4.5"
          }
        ]
      }
    }
  ]
}
```

## Context Injection

The pipeline uses **context injection** to provide JOLT transforms with read-only access to request/response envelope data.

### Structure

```json
{
  "data": <normalized_data>,  // Transform operates on this
  "context": {                 // Read-only context
    "request_details": {
      "query_params": { "patient": ["PID123"] },
      "headers": { ... },
      "metadata": { ... }
    },
    "target_details": {
      "metadata": { "PatientID": "PID123" }
    }
  }
}
```

### Configuration

Enable context injection in middleware config:

```toml
[middleware.my_transform]
type = "transform"
[middleware.my_transform.options]
spec_path = "path/to/transform.json"
inject_context = true  # Enable context
```

### Benefits

- **Dynamic queries**: Access query parameters in transforms
- **Conditional logic**: Branch based on request context
- **Read-only**: Context cannot be modified, only `data` field is transformed
- **Backward compatible**: Defaults to `false` for existing transforms

## Example Request/Response

### Request

```bash
curl "http://localhost:8081/fhir/ImagingStudy?patient=PID156695" \
  -H "Accept: application/fhir+json"
```

### Response

```json
{
  "resourceType": "Bundle",
  "type": "searchset",
  "entry": [
    {
      "resource": {
        "resourceType": "ImagingStudy",
        "id": "1.2.826.0.1.3680043.9.7133.3280065491876470",
        "identifier": [
          {
            "system": "urn:dicom:uid"
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
        "description": "Mock CT Study",
        "endpoint": [
          {
            "resourceType": "Endpoint",
            "status": "active",
            "connectionType": {
              "system": "http://terminology.hl7.org/CodeSystem/endpoint-connection-type",
              "code": "hl7-fhir-rest"
            },
            "address": "/api/jmix?studyInstanceUid=1.2.826.0.1.3680043.9.7133.3280065491876470",
            "payloadType": [
              {
                "coding": [
                  {
                    "system": "http://hl7.org/fhir/resource-types",
                    "code": "ImagingStudy"
                  }
                ]
              }
            ]
          }
        ]
      }
    }
  ]
}
```

### JMIX Download URLs

The `endpoint[].address` field contains JMIX API URLs for downloading study files:

```
/api/jmix?studyInstanceUid=1.2.826.0.1.3680043.9.7133.3280065491876470
```

**Note:** The base URL for JMIX endpoints should be configured separately. The current implementation uses relative paths.

## Advanced Configuration

### Adding Authentication

Add authentication middleware to the pipeline:

```toml
[pipelines.imagingstudy_query]
middleware = [
    "jwt_auth",              # Add JWT authentication
    "imagingstudy_filter",
    # ... rest of pipeline
]

[middleware.jwt_auth]
type = "jwtauth"
[middleware.jwt_auth.options]
secret = "your-secret-key"
issuer = "https://your-issuer.com"
```

### Custom JMIX Base URL

To configure a custom JMIX base URL, modify the `enrich_with_jmix_urls.json` transform or use middleware to set it in `target_details`.

### Error Handling

The pipeline handles several error scenarios:

- **Missing patient parameter**: Returns empty Bundle with `total: 0`
- **Invalid path**: `imagingstudy_filter` returns 404
- **DICOM connection error**: Returns HTTP 500
- **Transform errors**: Configurable via `fail_on_error` option

## Troubleshooting

### No Results Returned

1. Check query parameters are correct: `?patient=PID123`
2. Verify DICOM backend is reachable (if not using mock)
3. Check logs: `tail -f ./tmp/harmony_test.log`

### Transform Errors

1. Validate JOLT specs with test data
2. Enable debug logging: `log_level = "debug"` in config
3. Check `inject_context` is enabled where needed

### JMIX URLs Not Generated

1. Verify `enrich_jmix_urls` middleware is in the chain
2. Check `inject_context = true` is set
3. Ensure DICOM response includes `StudyInstanceUID`

## Security Considerations

- Management API bound to localhost (127.0.0.1) for security
- FHIR endpoints exposed on all interfaces (0.0.0.0)
- **Production**: Add JWT or basic auth middleware
- **Production**: Configure TLS/SSL for encrypted connections
- **Production**: Validate and sanitize all query parameters

## Files Structure

```
examples/fhir_dicom/
├── config.toml                                    # Main configuration
├── pipelines/
│   └── fhir_imagingstudy.toml                     # Pipeline definition
├── transforms/
│   ├── query_to_target_details.json               # Query param extraction
│   ├── metadata_set_dimse_op.json                 # Set DIMSE operation
│   ├── fhir_to_dicom_params.json                  # Build DICOM query (context-aware)
│   ├── enrich_with_jmix_urls.json                 # Add JMIX URLs (context-aware)
│   └── dicom_to_imagingstudy_simple.json          # DICOM → FHIR (context-aware)
└── README.md                                      # This file
```

## Key Concepts

### metadata_transform vs transform

- **`metadata_transform`**: Operates on envelope metadata/target_details
  - Used to extract query parameters
  - Sets `target_details.metadata` fields
  - Does not use context injection

- **`transform`**: Operates on normalized_data
  - Used for data transformation
  - Can use context injection to access envelope data
  - Transforms the `data` field, context is read-only

### Target Details

`target_details` is an envelope field that contains backend request metadata:

```rust
pub struct TargetDetails {
    pub base_url: String,
    pub method: String,
    pub uri: String,
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub query_params: HashMap<String, Vec<String>>,
    pub metadata: HashMap<String, String>,  // ← Used for DICOM query params
}
```

The DICOM backend reads `target_details.metadata` to build C-FIND queries.

## References

- [FHIR R4 ImagingStudy](https://www.hl7.org/fhir/imagingstudy.html)
- [DICOM Standard](https://www.dicomstandard.org/)
- [JMIX API Documentation](https://aurabx.github.io/jmix/#/spec/api)
- [Harmony Documentation](../../docs/)

## Support

For issues or questions:

1. Check the [main documentation](../../docs/)
2. Review transform specifications in `transforms/`
3. Enable debug logging for detailed pipeline traces
4. Examine `./tmp/harmony_test.log` for error details
