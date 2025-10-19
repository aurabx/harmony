# DICOM Backend (SCU) Example

This example demonstrates using HTTP requests to trigger DICOM SCU (Service Class User) operations against a PACS system.

## What This Example Demonstrates

- HTTP to DICOM protocol translation
- DICOM SCU backend configuration
- DIMSE operations (C-FIND, C-MOVE, C-STORE)
- Triggering PACS queries via REST API

## Prerequisites

- **DICOM Server**: Orthanc PACS or compatible DICOM server
- **Default Configuration**: Expects DICOM server at `localhost:4242` with AE title `ORTHANC`

### Setting up Orthanc (Optional)

```bash
docker run -p 4242:4242 -p 8042:8042 --name orthanc \
  -e ORTHANC_AE_TITLE=ORTHANC \
  orthancteam/orthanc
```

## Configuration

- **Proxy ID**: `harmony-dicom-backend`
- **HTTP Listener**: `127.0.0.1:8085`
- **Endpoint Path**: `/trigger-dicom`
- **DICOM Backend**: `localhost:4242` (AE: `ORTHANC`, Local AE: `HARMONY_SCU`)
- **Log File**: `./tmp/harmony_dicom_backend.log`
- **Storage**: `./tmp`

## How to Run

1. Ensure your DICOM server (e.g., Orthanc) is running

2. From the project root, run:
   ```bash
   cargo run -- --config examples/dicom-backend/config.toml
   ```

3. The service will start and bind to `127.0.0.1:8085`

## Testing

### Trigger DICOM Query

```bash
# Send HTTP request to trigger DICOM operations
curl -X POST http://127.0.0.1:8085/trigger-dicom \
  -H "Content-Type: application/json" \
  -d '{
    "operation": "C-FIND",
    "query": {
      "PatientID": "*",
      "PatientName": "*",
      "StudyInstanceUID": ""
    }
  }'
```

### Example Operations

**C-FIND (Query):**
```bash
curl -X POST http://127.0.0.1:8085/trigger-dicom \
  -H "Content-Type: application/json" \
  -d '{
    "operation": "C-FIND",
    "level": "STUDY",
    "query": {
      "StudyInstanceUID": "1.2.3.4.5.6"
    }
  }'
```

**C-MOVE (Retrieve):**
```bash
curl -X POST http://127.0.0.1:8085/trigger-dicom \
  -H "Content-Type: application/json" \
  -d '{
    "operation": "C-MOVE",
    "destination": "HARMONY_SCU",
    "query": {
      "StudyInstanceUID": "1.2.3.4.5.6"
    }
  }'
```

## Expected Behavior

1. HTTP request is received with DICOM operation parameters
2. Harmony establishes DICOM association with configured PACS
3. Specified DIMSE operation (C-FIND, C-MOVE, etc.) is performed
4. Results are returned in HTTP response
5. Association is released

## Use Cases

- **RESTful DICOM Interface**: Expose DICOM operations via HTTP API
- **Study Retrieval**: Trigger C-MOVE operations from web applications
- **PACS Queries**: Query PACS for studies, series, or images via HTTP
- **Integration Bridge**: Connect modern HTTP clients to legacy DICOM systems

## Files

- `config.toml` - Main configuration with DICOM backend settings
- `pipelines/dicom-backend.toml` - Pipeline definition
- `tmp/` - Created at runtime for logs and temporary storage

## Troubleshooting

- **Connection Refused**: Ensure DICOM server is running on configured host/port
- **Association Rejected**: Verify AE titles match (ORTHANC and HARMONY_SCU)
- **Timeout Errors**: Check network connectivity and firewall settings
- **Invalid Query**: Ensure DICOM query attributes are properly formatted

## DICOM Operations Reference

### Supported DIMSE Operations

- **C-ECHO**: Verify connection
- **C-FIND**: Query for studies, series, images
- **C-MOVE**: Retrieve studies from PACS
- **C-STORE**: Send DICOM objects to PACS

### Query Levels

- **PATIENT**: Patient-level queries
- **STUDY**: Study-level queries
- **SERIES**: Series-level queries
- **IMAGE**: Image-level queries

## Next Steps

- See `examples/fhir-to-dicom/` for FHIR to DICOM translation
- Explore `examples/dicom-scp/` to receive DICOM connections
- Review DICOM standard for query/retrieve specifications
