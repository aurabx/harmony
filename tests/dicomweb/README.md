# DICOMweb Integration Tests

This directory contains integration tests for the `dicom_to_dicomweb` middleware.

## Test Structure

### Unit Tests (`dicom_to_dicomweb_middleware.rs`)
Tests the middleware transformation logic in isolation:
- C-FIND → QIDO-RS transformation
- C-STORE → STOW-RS transformation
- C-GET → WADO-RS transformation
- C-MOVE → WADO-RS transformation

Run with:
```bash
cargo test --test dicom_to_dicomweb_middleware
```

### Integration Tests (`dicom_to_dicomweb_integration.rs`)
Full end-to-end tests using real DCMTK tools and a mock DICOMweb server:
- Tests the complete pipeline: DCMTK → DICOM SCP → middleware → HTTP Backend
- Uses actual DICOM protocol communication
- Validates request/response transformation

## Prerequisites

### 1. Install DCMTK Tools
```bash
# macOS
brew install dcmtk

# Ubuntu/Debian
sudo apt-get install dcmtk

# Verify installation
which echoscu storescu findscu getscu movescu
```

### 2. Python 3 (for mock server)
```bash
python3 --version
```

### 3. Sample DICOM Files
The tests use sample DICOM files from `samples/dicom/study_1/`. These should be present in the repository.

## Running Integration Tests

### Run All Integration Tests
```bash
# Run with --ignored flag (integration tests are marked with #[ignore])
cargo test --test dicom_to_dicomweb_integration -- --ignored --nocapture
```

### Run Individual Tests
```bash
# C-ECHO test
cargo test --test dicom_to_dicomweb_integration test_dicom_c_echo -- --ignored --nocapture

# C-STORE → STOW-RS
cargo test --test dicom_to_dicomweb_integration test_dicom_c_store_to_stow_rs -- --ignored --nocapture

# C-FIND → QIDO-RS
cargo test --test dicom_to_dicomweb_integration test_dicom_c_find_to_qido_rs -- --ignored --nocapture

# C-GET → WADO-RS
cargo test --test dicom_to_dicomweb_integration test_dicom_c_get_to_wado_rs -- --ignored --nocapture

# C-MOVE → WADO-RS
cargo test --test dicom_to_dicomweb_integration test_dicom_c_move_to_wado_rs -- --ignored --nocapture

# Full workflow (STORE → FIND → GET)
cargo test --test dicom_to_dicomweb_integration test_full_workflow -- --ignored --nocapture
```

## Mock DICOMweb Server

The integration tests automatically start a mock DICOMweb server (`mock_dicomweb_server.py`).

### Running the Mock Server Manually
```bash
# Start on default port (8042)
./tests/dicomweb/mock_dicomweb_server.py

# Start on custom port
./tests/dicomweb/mock_dicomweb_server.py 8043
```

### Supported Endpoints
- **QIDO-RS** (Query):
  - `GET /studies` - Query all studies
  - `GET /studies?PatientID=TEST123` - Query with filter
  - `GET /studies/{uid}/series` - Query series
  - `GET /studies/{uid}/series/{uid}/instances` - Query instances

- **STOW-RS** (Store):
  - `POST /studies` - Store DICOM instances

- **WADO-RS** (Retrieve):
  - `GET /studies/{uid}` - Retrieve study
  - `GET /studies/{uid}/series/{uid}` - Retrieve series
  - `GET /studies/{uid}/series/{uid}/instances/{uid}` - Retrieve instance

## Test Architecture

```
┌──────────────┐
│ DCMTK Tools  │ (storescu, findscu, getscu, movescu)
└──────┬───────┘
       │ DICOM Protocol
       ▼
┌──────────────┐
│  DICOM SCP   │ (Harmony endpoint on port 11113)
│  Endpoint    │
└──────┬───────┘
       │ Internal Envelope
       ▼
┌──────────────────────┐
│ dicom_to_dicomweb    │ (Middleware)
│    Middleware        │
└──────┬───────────────┘
       │ HTTP Request
       ▼
┌──────────────────────┐
│   HTTP Backend       │ (Harmony HTTP backend)
└──────┬───────────────┘
       │ HTTP
       ▼
┌──────────────────────┐
│ Mock DICOMweb Server │ (Python, port 8043)
└──────────────────────┘
```

## Troubleshooting

### DICOM Association Timeout
If tests fail with "DUL network read timeout", this indicates the DICOM SCP is accepting TCP connections but failing to complete association negotiation. This may be due to:
- Issues in the underlying `dimse` crate's association handling
- Incompatible presentation context negotiation
- SCP not implementing the required SOP classes

To debug:
```bash
# Enable detailed tracing
RUST_LOG=dimse=trace,harmony=debug cargo test --test dicom_to_dicomweb_integration -- --ignored --nocapture
```

### Tests Skip with "DCMTK tools not found"
Install DCMTK tools as described in Prerequisites.

### Mock Server Fails to Start
- Check Python 3 is installed
- Ensure port 8043 is not already in use
- Check for errors in test output

### DICOM Port Already in Use
The tests use port 11113. If this port is in use, you can modify the port in `dicom_to_dicomweb_integration.rs`:
```rust
let dicom_port = 11113; // Change this
```

### View Detailed Logs
Integration tests create logs at:
```
./tmp/dicom_to_dicomweb_integration.log
```

### Enable DCMTK Debug Output
Add `-d` flag to DCMTK commands in the test code for more verbose output.

## Known Issues

### DIMSE SCP Association (as of Nov 2025)
The internal DIMSE SCP implementation (from the `dimse` crate) has issues completing
DICOM association negotiation with DCMTK tools. The SCP accepts TCP connections
but fails to respond properly to association requests.

The middleware implementation (`dicom_to_dicomweb`) is complete and unit-tested.
Once the underlying DIMSE SCP issues are resolved, these integration tests should pass.

## CI/CD Considerations

These tests are marked with `#[ignore]` because they require:
1. External tools (DCMTK)
2. Python 3
3. Network ports
4. Sample DICOM files

For CI/CD, you would need to:
1. Install DCMTK in the CI environment
2. Ensure Python 3 is available
3. Include sample DICOM files in the repository or test artifacts

## Future Enhancements

- [ ] Add more comprehensive validation of DICOMweb responses
- [ ] Test error cases (invalid UIDs, missing data, etc.)
- [ ] Add performance benchmarks
- [ ] Test with real DICOMweb servers (Orthanc, DCM4CHEE)
- [ ] Add Docker-based test environment for CI/CD
- [ ] Test concurrent requests
- [ ] Validate multipart MIME handling in detail
