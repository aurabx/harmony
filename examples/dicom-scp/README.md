# DICOM SCP Example

**Implementation Status:**
- ✅ C-ECHO (Verification)
- ✅ C-FIND (Query)
- ✅ C-GET (Retrieve)
- ✅ C-MOVE (Move)
- ⏳ C-STORE (Storage) - **NOT YET IMPLEMENTED**

This example demonstrates a DICOM SCP (Service Class Provider) endpoint that accepts incoming DIMSE connections for query and retrieve operations.

## What This Example Demonstrates

- DICOM SCP endpoint configuration
- Accepting DIMSE associations
- C-ECHO verification service
- C-FIND query operations
- C-GET retrieve operations  
- C-MOVE move operations
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

### Using DCMTK echoscu (C-ECHO)

```bash
# Verify connectivity
echoscu -aec HARMONY_SCP 127.0.0.1 11112
```

### Using DCMTK findscu (C-FIND)

```bash
# Query for all patients
findscu -aec HARMONY_SCP -P 127.0.0.1 11112 -k 0010,0020="*"

# Query for specific patient
findscu -aec HARMONY_SCP -P 127.0.0.1 11112 -k 0010,0020="PATIENT123"

# Study-level query
findscu -aec HARMONY_SCP -S 127.0.0.1 11112 -k 0020,000D="*"
```

### Using DCMTK getscu (C-GET)

```bash
# Retrieve study
getscu -aec HARMONY_SCP 127.0.0.1 11112 -k 0020,000D="1.2.3.4.5"
```

### Using DCMTK movescu (C-MOVE)

```bash
# Move study to destination AET
movescu -aec HARMONY_SCP -aem DEST_AET 127.0.0.1 11112 -k 0020,000D="1.2.3.4.5"
```

### C-STORE (Not Yet Implemented)

```bash
# This will NOT work yet - C-STORE is not implemented
# storescu -aec HARMONY_SCP 127.0.0.1 11112 /path/to/file.dcm
```

### Using Orthanc

Configure Orthanc to query/retrieve from Harmony:

```json
{
  "DicomModalities": {
    "harmony": ["HARMONY_SCP", "127.0.0.1", 11112]
  }
}
```

Then query via Orthanc UI or API:
```bash
# Query for studies
curl -X POST http://localhost:8042/modalities/harmony/query \
  -d '{"Level":"Study","Query":{"PatientID":"*"}}'
```

**Note:** C-STORE to Harmony is not yet supported. Use query/retrieve operations instead.

## Expected Behavior

### C-ECHO
1. DICOM client establishes association with `HARMONY_SCP`
2. Client sends C-ECHO request
3. Harmony SCP responds with success
4. Association is released

### C-FIND
1. DICOM client establishes association
2. Client sends C-FIND query request
3. Harmony SCP searches and returns matching results
4. Results streamed back to client
5. Association is released

### C-GET / C-MOVE
1. DICOM client establishes association
2. Client sends C-GET/C-MOVE request with identifier
3. Harmony SCP retrieves matching datasets
4. Datasets sent back to client (C-GET) or forwarded to destination (C-MOVE)
5. Association is released

All events are logged to `./tmp/harmony_dicom_scp.log`

## Use Cases

- **DICOM Query Server**: Respond to C-FIND queries from PACS/workstations
- **Retrieve Service**: Provide C-GET/C-MOVE access to stored studies
- **DICOM Router**: Query and route studies between systems
- **Testing Tool**: Validate DICOM SCU client implementations
- **Future - PACS Storage**: Will support C-STORE when implemented

## Storage

Currently, the SCP queries and retrieves from configured backends/data sources. When C-STORE is implemented, received DICOM files will be stored in:
```
./tmp/dimse/
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
- **SOP Classes Supported**:
  - Verification SOP Class (C-ECHO)
  - Patient Root Query/Retrieve (C-FIND, C-GET, C-MOVE)
  - Study Root Query/Retrieve (C-FIND, C-GET, C-MOVE)
  - Storage SOP Classes (C-STORE) - Coming soon

## Verification

Verify the SCP is working:

```bash
# Test connectivity
echoscu -aec HARMONY_SCP 127.0.0.1 11112

# Test query
findscu -aec HARMONY_SCP -P 127.0.0.1 11112 -k 0010,0020="*"

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
