# FHIR to DICOM Translation Example

This example demonstrates translating FHIR resources to DICOM operations via HTTP triggers and backend DICOM SCU operations.

## What This Example Demonstrates

- HTTP to DICOM protocol translation
- Basic authentication and JSON extraction
- DICOM backend (SCU) integration
- Pipeline-based data transformation

## Prerequisites

- **DICOM Server**: Orthanc PACS or compatible DICOM server
- **Default Configuration**: Expects DICOM server at `127.0.0.1:4242` with AE title `PACSCLINIC`

### Setting up Orthanc (Optional)

```bash
docker run -p 4242:4242 -p 8042:8042 --name orthanc \
  -e ORTHANC_AE_TITLE=PACSCLINIC \
  orthancteam/orthanc
```

## Configuration

- **Proxy ID**: `harmony-fhir-to-dicom`
- **HTTP Listener**: `127.0.0.1:8082`
- **Endpoint Path**: `/test`
- **DICOM Backend**: `127.0.0.1:104` (AE: `PACSCLINIC`, Local AE: `HM_DICOM_A`)
- **Log File**: `./tmp/harmony_fhir_to_dicom.log`
- **Storage**: `./tmp`

## How to Run

1. Ensure your DICOM server (e.g., Orthanc) is running

2. From the project root, run:
   ```bash
   cargo run -- --config examples/fhir-to-dicom/config.toml
   ```

3. The service will start and bind to `127.0.0.1:8082`

## Testing

```bash
# Send FHIR data that will be translated to DICOM
curl -X POST http://127.0.0.1:8082/test \
  -u test_user:test_password \
  -H "Content-Type: application/json" \
  -d '{
    "resourceType": "ImagingStudy",
    "id": "example",
    "patient": {
      "reference": "Patient/example"
    },
    "started": "2024-01-15T10:30:00Z"
  }'
```

## Expected Behavior

1. HTTP request is received with FHIR data
2. Basic authentication validates credentials
3. JSON is extracted and normalized
4. Data flows through the pipeline
5. DICOM backend (SCU) performs DIMSE operations against the configured PACS

## Transform Files

- `transforms/fhir_to_dicom_params.json` - Maps FHIR data to DICOM parameters
- `transforms/dicom_to_imagingstudy_simple.json` - Converts DICOM study data to FHIR ImagingStudy

## Files

- `config.toml` - Main configuration with DICOM backend settings
- `pipelines/fhir-dicom.toml` - Pipeline definition
- `transforms/` - JOLT transformation specifications
- `tmp/` - Created at runtime for logs and temporary storage

## Troubleshooting

- **Connection Refused**: Ensure DICOM server is running on the configured host/port
- **Association Rejected**: Verify AE titles match between Harmony and the DICOM server
- **Authentication Errors**: Check username/password in the curl command

## Next Steps

- See `examples/dicom-backend/` for direct HTTP to DICOM SCU examples
- Explore `examples/transform/` to understand JOLT transformations
