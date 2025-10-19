# JMIX High-Performance Example

This example demonstrates the JMIX endpoint with performance optimization flags for handling large datasets efficiently.

## What This Example Demonstrates

- JMIX endpoint configuration
- Performance optimization with skip flags
- DICOM backend integration for data retrieval
- Configurable hashing and file listing behavior
- Storage and retrieval of JMIX envelopes

## Prerequisites

- **DICOM Server**: Orthanc PACS or compatible DICOM server at `127.0.0.1:4242`
- **JMIX Schema**: Schema directory at `../jmix` (configurable)

### Setting up Orthanc (Optional)

```bash
docker run -p 4242:4242 -p 8042:8042 --name orthanc \
  -e ORTHANC_AE_TITLE=ORTHANC \
  orthancteam/orthanc
```

## Configuration

- **Proxy ID**: `harmony-jmix`
- **HTTP Listener**: `127.0.0.1:8084`
- **Endpoint Path**: `/jmix`
- **DICOM Backend**: `127.0.0.1:4242` (AE: `ORTHANC`, Local AE: `HARMONY_SCU`)
- **Performance Options**:
  - `skip_hashing`: `true` (skips SHA256 hash computation for speed)
  - `skip_listing`: `false` (includes DICOM files in manifest)
- **Log File**: `./tmp/harmony_jmix.log`
- **Storage**: `./tmp`

## How to Run

1. Ensure your DICOM server (e.g., Orthanc) is running

2. From the project root, run:
   ```bash
   cargo run -- --config examples/jmix/config.toml
   ```

3. The service will start and bind to `127.0.0.1:8084`

## API Endpoints

### 1. Create JMIX Envelope

```bash
# Request JMIX envelope for a study
curl "http://127.0.0.1:8084/jmix/api/jmix?studyInstanceUid=1.2.3.4.5.6"
```

**Query Parameters:**
- `studyInstanceUid`: DICOM Study Instance UID (required)
- `skip_hashing`: Override config default (optional, true/false)
- `skip_listing`: Override config default (optional, true/false)

### 2. Download Stored Envelope

```bash
# Download as ZIP
curl "http://127.0.0.1:8084/jmix/api/jmix/{envelope_id}" \
  -H "Accept: application/zip" \
  -o envelope.zip

# Download as GZIP
curl "http://127.0.0.1:8084/jmix/api/jmix/{envelope_id}" \
  -H "Accept: application/gzip" \
  -o envelope.tar.gz
```

### 3. Fetch Manifest

```bash
# Get manifest.json for a stored envelope
curl "http://127.0.0.1:8084/jmix/api/jmix/{envelope_id}/manifest"
```

## Expected Behavior

1. Client requests JMIX envelope for a study via Study Instance UID
2. Harmony performs C-FIND and C-MOVE operations against DICOM PACS
3. Retrieved DICOM files are processed and packaged into JMIX format
4. Envelope is stored with unique ID
5. Client can download envelope or retrieve manifest

## Performance Optimization

### Skip Hashing (`skip_hashing = true`)

- **Default**: `true` in this example
- **Effect**: Skips SHA256 hash computation for DICOM files
- **Benefit**: Significantly faster processing for large studies
- **Trade-off**: No file integrity verification via hash

### Skip Listing (`skip_listing = false`)

- **Default**: `false` in this example
- **Effect**: Includes full file listing in `files.json` manifest
- **Benefit**: Complete inventory of DICOM files
- **Trade-off**: Slight performance overhead

### When to Use Performance Flags

- **High-throughput scenarios**: Enable both skip flags
- **Data integrity critical**: Disable `skip_hashing`
- **Need file inventory**: Keep `skip_listing = false`
- **Maximum speed**: Set both to `true`

## Files

- `config.toml` - Main configuration with performance options
- `pipelines/jmix-performance.toml` - Pipeline with JMIX endpoint
- `tmp/` - Created at runtime for logs, storage, and JMIX envelopes

## Troubleshooting

- **Connection Refused**: Ensure DICOM server is running on configured host/port
- **No Studies Found**: Verify Study Instance UID exists in PACS
- **C-STORE Failures**: Check that `incoming_store_port` (11112) is available

## Next Steps

- Explore `examples/dicomweb/` for DICOMweb protocol support
- See `examples/jmix-to-dicom/` for JMIX to DICOM workflows
- Review JMIX schema specifications in `../jmix` directory
