# DIMSE Crate Test Coverage

This document describes the comprehensive test suite for the `dimse` crate.

## Test Overview

The dimse crate has **26 total tests** covering unit and integration testing:

- **15 unit tests** - Internal functionality tests
- **5 SCP integration tests** - End-to-end Service Class Provider tests
- **6 SCU integration tests** - End-to-end Service Class User tests

## Running Tests

```bash
# Run all tests
cargo test

# Run only unit tests
cargo test --lib

# Run only SCP integration tests
cargo test --test scp_integration

# Run only SCU integration tests
cargo test --test scu_integration

# Run with logging output
RUST_LOG=debug cargo test -- --nocapture
```

## Unit Tests (15 tests)

Located in `src/` modules with `#[cfg(test)]` blocks:

### Config Tests (3)
- `test_default_config` - Validates default configuration values
- `test_config_validation` - Tests configuration validation rules
- `test_remote_node_builder` - Tests remote node builder pattern

### SCP Tests (2)
- `test_scp_creation` - Tests SCP instantiation
- `test_default_query_provider` - Tests default query provider

### SCU Tests (5)
- `test_scu_creation` - Tests SCU instantiation
- `test_echo_stub` - Tests C-ECHO stub (ignored unless feature enabled)
- `test_find_stub` - Tests C-FIND stub
- `test_connection_timeout_selection` - Tests timeout configuration
- `test_invalid_config_validation` - Tests invalid configuration rejection

### Router Tests (2)
- `test_router_echo` - Tests echo command routing
- `test_request_builders` - Tests request builder pattern

### Types Tests (3)
- `test_dataset_metadata` - Tests dataset metadata creation
- `test_find_query_builder` - Tests C-FIND query builder
- `test_query_level_parsing` - Tests DICOM query level parsing

## SCP Integration Tests (5 tests)

Located in `tests/scp_integration.rs`:

### test_scp_starts_and_stops
Tests SCP lifecycle management:
- Allocates ephemeral port
- Starts SCP in background task
- Verifies port is listening
- Triggers graceful shutdown
- Confirms shutdown completes within 2 seconds

**Status**: ✅ Passes

### test_scp_accepts_c_echo
Tests C-ECHO connectivity with DCMTK:
- Starts SCP with C-ECHO enabled
- Runs DCMTK `echoscu` to test connectivity
- Verifies C-ECHO success response
- Cleans up gracefully

**Requirements**: DCMTK `echoscu` tool
**Status**: ✅ Passes (skips if DCMTK not available)

### test_scp_accepts_c_find
Tests C-FIND query handling with DCMTK:
- Starts SCP with C-FIND enabled
- Runs DCMTK `findscu` with wildcard query
- Verifies C-FIND completes successfully
- Validates empty result set handling

**Requirements**: DCMTK `findscu` tool
**Status**: ✅ Passes (skips if DCMTK not available)

### test_scp_config_validation
Tests configuration validation rules:
- Rejects AE titles longer than 16 characters
- Ensures proper error messages

**Status**: ✅ Passes

### test_scp_multiple_associations
Tests concurrent association handling:
- Starts SCP with max_associations=5
- Runs 3 concurrent `echoscu` commands
- Verifies all associations succeed
- Tests thread safety

**Requirements**: DCMTK `echoscu` tool
**Status**: ✅ Passes (skips if DCMTK not available)

## SCU Integration Tests (6 tests)

Located in `tests/scu_integration.rs`:

### test_scu_echo_success
Tests SCU C-ECHO functionality:
- Creates remote node configuration
- Attempts C-ECHO to remote SCP
- Expected to fail in CI unless SCP is running
- Validates error handling

**Note**: This test is informational - expects failure unless a test PACS is available
**Status**: ✅ Passes (handles expected failure)

### test_scu_config_validation
Tests SCU configuration validation:
- Rejects empty AE title
- Rejects AE title longer than 16 characters
- Rejects empty host
- Rejects port 0

**Status**: ✅ Passes

### test_scu_connection_timeout
Tests connection timeout behavior:
- Configures 500ms timeout
- Attempts connection to non-routable address (192.0.2.1)
- Verifies timeout/connection failure

**Status**: ✅ Passes

### test_scu_find_query_builder
Tests C-FIND query builder API:
- Patient-level queries with parameters
- Study-level queries with filters
- Parameter mapping (PatientID, PatientName, StudyDate)
- Max results configuration

**Status**: ✅ Passes

### test_scu_move_query_builder
Tests C-MOVE query builder API:
- Query level configuration
- Destination AET specification
- Priority setting (High/Medium/Low)
- Parameter handling

**Status**: ✅ Passes

### test_scu_get_query_builder
Tests C-GET query builder API:
- Series-level queries
- Parameter mapping
- Query construction

**Status**: ✅ Passes

## Test Coverage Summary

### What's Tested
- ✅ SCP starts and stops gracefully
- ✅ SCP accepts C-ECHO from DCMTK tools
- ✅ SCP accepts C-FIND from DCMTK tools
- ✅ SCP handles multiple concurrent associations
- ✅ SCP configuration validation
- ✅ SCU configuration validation
- ✅ SCU connection timeouts
- ✅ Query builder APIs (FIND, MOVE, GET)
- ✅ Dataset metadata handling
- ✅ DICOM query level parsing

### What's NOT Tested (Future Work)
- ❌ C-MOVE actual retrieval operations
- ❌ C-GET actual retrieval operations
- ❌ C-STORE operations
- ❌ TLS/encryption
- ❌ Query provider with real data
- ❌ DICOMweb integration
- ❌ Performance/load testing
- ❌ Association negotiation edge cases

## DCMTK Compatibility

The integration tests use DCMTK tools for end-to-end validation:
- `echoscu` - C-ECHO verification
- `findscu` - C-FIND query testing
- `dcmqrscp` - Optional test PACS (for SCU tests)

Tests automatically skip if DCMTK is not installed.

### Installing DCMTK

**macOS**:
```bash
brew install dcmtk
```

**Ubuntu/Debian**:
```bash
sudo apt-get install dcmtk
```

**From source**:
See https://dicom.offis.de/dcmtk

## CI/CD Considerations

- Unit tests run in all environments (no dependencies)
- SCP integration tests require DCMTK but gracefully skip if unavailable
- SCU integration tests may show expected failures in CI
- All tests complete in < 1 second except SCP tests (< 300ms each)

## Test Data

Tests use:
- Ephemeral ports to avoid conflicts
- `/tmp/dimse_test` for temporary storage
- Mock query providers (no real DICOM data required)
- DCMTK command-line tools for protocol validation

## Adding New Tests

When adding new functionality to the dimse crate:

1. **Add unit tests** in the relevant `src/*.rs` module
2. **Add integration tests** in `tests/` for end-to-end validation
3. **Use MockQueryProvider** for isolated SCP tests
4. **Use DCMTK tools** for real protocol validation when possible
5. **Skip tests gracefully** if external dependencies unavailable
6. **Document test requirements** in this file

## Test Maintenance

Keep tests fast, isolated, and deterministic:
- ✅ Use ephemeral ports
- ✅ Clean up resources
- ✅ Use timeouts to prevent hangs
- ✅ Skip tests with missing dependencies
- ✅ Test both success and failure paths
- ✅ Validate error messages
