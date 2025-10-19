# DICOM SCP Example

This example demonstrates a DICOM SCP (Service Class Provider) endpoint that accepts incoming DIMSE connections and C-STORE operations.

## What This Example Demonstrates

- DICOM SCP endpoint configuration
- Accepting DIMSE associations
- Receiving C-STORE operations
- Storage of received DICOM objects
- DICOM network listener setup

## Prerequisites

- **DICOM Client**: DCMTK tools (`storescu`) or Orthanc for sending DICOM files
- **Port Availability**: Port 11112 must be available for DICOM listener

### Installing DCMTK (Optional)

**macOS:**
```bash
brew install dcmtk
```

**Linux:**
```bash
apt-get install dcmtk
```

## Configuration

- **Proxy ID**: `harmony-dicom-scp`
- **DICOM Listener**: `0.0.0.0:11112`
- **AE Title**: `HARMONY_SCP`
- **Log File**: `./tmp/harmony_dicom_scp.log`
- **Storage**: `./tmp` (received DICOM files stored here)

## How to Run

1. From the project root, run:
   ```bash
   cargo run -- --config examples/dicom-scp/config.toml
   ```

2. The service will start and bind DICOM listener to `0.0.0.0:11112`

## Testing

### Using DCMTK storescu

```bash
# Send a single DICOM file
storescu -aec HARMONY_SCP 127.0.0.1 11112 /path/to/file.dcm

# Send multiple files
storescu -aec HARMONY_SCP 127.0.0.1 11112 /path/to/dicom/directory/

# With verbose output
storescu -v -aec HARMONY_SCP 127.0.0.1 11112 /path/to/file.dcm
```

### Using Orthanc

Configure Orthanc to push studies to Harmony:

```json
{
  "DicomModalities": {
    "harmony": ["HARMONY_SCP", "127.0.0.1", 11112]
  }
}
```

Then send via Orthanc UI or API:
```bash
curl -X POST http://localhost:8042/modalities/harmony/store \
  -d '{"Resources":["study-id-here"]}'
```

## Expected Behavior

1. DICOM client establishes association with `HARMONY_SCP`
2. Client sends C-STORE request with DICOM object
3. Harmony SCP accepts the object
4. Object is stored in `./tmp` directory
5. Association is released
6. Event is logged to `./tmp/harmony_dicom_scp.log`

## Use Cases

- **PACS Storage Node**: Act as a DICOM storage destination
- **DICOM Router**: Accept studies and forward to other destinations
- **Archive Endpoint**: Store received DICOM objects for processing
- **Testing Tool**: Validate DICOM client implementations

## Stored Files

Received DICOM files are stored in:
```
./tmp/
├── [StudyInstanceUID]/
│   └── [SeriesInstanceUID]/
│       └── [SOPInstanceUID].dcm
```

## Troubleshooting

- **Port Already in Use**: Change `bind_port` in config or free up port 11112
- **Association Rejected**: Verify client uses correct AE title (`HARMONY_SCP`)
- **Connection Timeout**: Check firewall settings and network connectivity
- **Permission Denied**: Ensure write permissions for `./tmp` directory

## DICOM Association Parameters

- **Called AE Title**: `HARMONY_SCP`
- **Maximum PDU Size**: 16384 bytes (default)
- **Transfer Syntaxes**: ImplicitVRLittleEndian, ExplicitVRLittleEndian
- **SOP Classes**: Storage SOP classes (CT, MR, US, etc.)

## Verification

Check that files were received:

```bash
# List received files
ls -lR ./tmp/

# Validate DICOM file
dcmdump ./tmp/[study]/[series]/[instance].dcm

# Check logs
tail -f ./tmp/harmony_dicom_scp.log
```

## Files

- `config.toml` - Main configuration with DICOM SCP listener settings
- `pipelines/dicom-scp.toml` - Pipeline definition
- `tmp/` - Created at runtime for logs and received DICOM files

## Next Steps

- See `examples/dicom-backend/` for DICOM SCU (client) operations
- Explore `examples/jmix/` for packaging DICOM studies into JMIX format
- Review DICOM standard for storage SOP class specifications
