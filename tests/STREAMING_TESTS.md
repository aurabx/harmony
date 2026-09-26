# Streaming Integration Tests

## Overview

Comprehensive test suite for DICOM streaming functionality in Harmony Proxy.

## Test Files

### 1. `streaming_integration.rs` - Configuration & Detection Tests

**Status**: ✅ All tests passing (8/8)

**What's tested:**
- Streaming pipeline detection logic
- Service type registration (dicom_scu_streaming, http_streaming_backend)
- StreamFormat parsing (SSE, NDJSON, Multipart)
- Configuration validation for streaming components
- Bidirectional proxy-to-proxy setup
- Backward compatibility with non-streaming pipelines

**Run:**
```bash
cargo test --test streaming_integration
```

**Tests:**
- `test_streaming_pipeline_detection` - Verifies PipelineExecutor correctly identifies streaming pipelines
- `test_http_streaming_backend_detection` - Tests http_streaming_backend service detection
- `test_stream_format_from_config` - Validates format string parsing
- `test_stream_format_from_accept_header` - Tests Accept header parsing
- `test_dicom_scu_streaming_backend_config` - DICOM backend configuration validation
- `test_http_streaming_endpoint_config` - HTTP endpoint configuration validation
- `test_bidirectional_streaming_config` - Complete proxy-to-proxy setup
- `test_backward_compatibility` - Ensures non-streaming pipelines still work

### 2. `streaming_dicom_dcmtk.rs` - DICOM Integration Tests

**Status**: ✅ Self-contained (only requires dcmtk installed)

**What's tested:**
- Real DICOM operations using dcmtk tools (echoscu, findscu, getscu, storescu)
- Spawns dcmqrscp (DICOM QR SCP) automatically
- C-ECHO, C-FIND, C-GET operations

**Prerequisites:**
- dcmtk tools installed (echoscu, findscu, getscu, storescu, dcmqrscp)

**Install dcmtk:**
```bash
# macOS
brew install dcmtk

# Ubuntu/Debian
sudo apt-get install dcmtk
```

**Run self-contained tests:**
```bash
# These spawn dcmqrscp and run automatically
cargo test --test streaming_dicom_dcmtk
```

**Show setup instructions:**
```bash
cargo test --test streaming_dicom_dcmtk test_setup_instructions -- --nocapture
```

**Self-Contained Tests (no Harmony required):**
- `test_dicom_echo_to_qrscp` - C-ECHO connectivity test with spawned dcmqrscp
- `test_dicom_find_query` - C-FIND query test with spawned dcmqrscp
- `test_dicom_get_retrieve` - C-GET retrieve test with spawned dcmqrscp

### 3. Tests that need a running Harmony

These are htest examples in the harmony-examples repository. htest starts Harmony and every service the test declares.

- `pipelines/dicom-scp` covers Harmony as a DICOM SCP (C-ECHO, C-FIND, C-GET, C-MOVE, C-STORE).
- `pipelines/streaming-dicom` covers end-to-end streaming through two Harmony instances. It is skipped until AURA-2408 is fixed.

```bash
# From the harmony-examples checkout
htest --examples-dir . run dicom-scp
htest --examples-dir . run streaming-dicom
```

The workspace `docs/testing.md` describes htest and harmony-runner.

## Quick Test Summary

### Running All Tests

```bash
# Unit tests (no external dependencies)
cargo test --test streaming_integration

# Self-contained DICOM tests (only requires dcmtk)
cargo test --test streaming_dicom_dcmtk

# All streaming tests
cargo test streaming
```

### Test Results

**Unit Tests**: ✅ 8/8 passing  
**DICOM Tests (self-contained)**: ✅ 3/3 passing

## What's Covered

### ✅ Fully Tested (No External Services)
- Streaming pipeline detection
- Service registration and configuration
- Format parsing and detection (SSE, NDJSON, Multipart)
- Configuration validation
- Backward compatibility
- Live DICOM operations (C-ECHO, C-FIND, C-GET with spawned dcmqrscp)

### Covered by htest examples (Harmony running)
- HTTP stream consumption via HTTP endpoints
- Per-instance middleware application through proxy
- End-to-end proxy streaming (DIMSE → HTTP → DIMSE), in `streaming-dicom` (skipped until AURA-2408 is fixed)

### 🚧 Not Yet Tested (Future Work)
- Performance/memory benchmarks
- Concurrent stream handling
- Network failure recovery
- Large study streaming (100+ instances)
- Transform middleware on streamed instances
- Authentication on streaming endpoints

## Test Data

For integration tests with actual DICOM data, you can:

1. **Use Orthanc's sample data:**
   ```bash
   # Upload sample DICOM via Orthanc REST API
   curl -X POST http://localhost:8042/tools/create-dicom -d '{
     "PatientName": "Test^Patient",
     "StudyInstanceUID": "1.2.3.4.5.6.7.8.9",
     "Modality": "CT"
   }'
   ```

2. **Use dcmtk to generate test files:**
   ```bash
   # Create a test DICOM file
   dcmodify --insert "PatientName=Test^Patient" input.dcm output.dcm
   ```

3. **Upload existing DICOM files:**
   ```bash
   # Via Orthanc REST API
   curl -X POST -H "Content-Type: application/dicom" \
     --data-binary @sample.dcm \
     http://localhost:8042/instances
   ```

## Troubleshooting

### Self-Contained Tests Fail

**Problem**: Tests skip with "dcmtk tools not found"

**Solution**: Install dcmtk:
```bash
# macOS
brew install dcmtk

# Ubuntu/Debian
sudo apt-get install dcmtk

# Verify installation
which echoscu findscu getscu storescu dcmqrscp
```


## CI/CD Integration

### Self-Contained Tests (No Setup Required)

The self-contained tests can run in any CI environment with dcmtk installed:

```yaml
# .github/workflows/streaming-tests.yml
name: Streaming Tests
on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Install dcmtk
        run: sudo apt-get update && sudo apt-get install -y dcmtk
      
      - name: Run self-contained tests
        run: cargo test --test streaming_dicom_dcmtk
```

### Tests That Need Harmony

Run these through htest in harmony-examples. See the workspace `docs/testing.md`.

## Performance Testing

For performance benchmarks (memory, latency, throughput):

```bash
# TODO: Create performance test suite
# cargo test --test streaming_performance -- --ignored
```

Expected results (from docs/streaming.md):
- Memory: 12MB peak (6 instances in flight) vs 1.2GB buffered
- Latency: 80ms until first instance vs 28s buffered
- Throughput: 120 instances/sec vs 40 instances/sec buffered

## Contributing

When adding new streaming features:

1. Add configuration tests to `streaming_integration.rs`
2. Add self-contained dcmtk tests to `streaming_dicom_dcmtk.rs`. Add tests that need a running Harmony as htest examples in harmony-examples.
3. Update this documentation
4. Ensure all unit tests pass: `cargo test --test streaming_integration`

## References

- [Streaming Architecture](../docs/streaming.md) - Complete architecture guide
- [Example Configuration](../examples/dicom-streaming/) - Sample configs
- [DICOM Backend Docs](../docs/backends.md) - Backend documentation
- [dcmtk Documentation](https://dicom.offis.de/dcmtk.php.en) - DICOM toolkit
- [Orthanc Documentation](https://www.orthanc-server.com/static.php?page=documentation) - PACS server
