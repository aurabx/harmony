# DIMSE Crate Test Coverage

This document describes the comprehensive test suite for the `dimse` crate.

## Test Overview

The dimse crate has **55+ total tests** covering unit and integration testing:

- **38 unit tests** - Internal functionality tests
- **11 SCP integration tests** - End-to-end Service Class Provider tests
- **16 SCU integration tests** - End-to-end Service Class User tests

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

## Unit Tests (38 tests)

Located in `src/` modules with `#[cfg(test)]` blocks:

### Config Tests (3)
- `test_default_config` - Validates default configuration values
- `test_config_validation` - Tests configuration validation rules
- `test_remote_node_builder` - Tests remote node builder pattern

### Common Module Tests

#### Message Builder Tests (4)
- `test_build_request` - Tests DIMSE request building
- `test_build_response` - Tests DIMSE response building
- `test_builder_with_sub_operations` - Tests sub-operation counts in responses
- `test_encode_command` - Tests command encoding to bytes

#### Query Utils Tests (2)
- `test_normalize_tag` - Tests DICOM tag normalization
- `test_query_level_to_string` - Tests query level string conversion

### SCP Tests

#### Response Builder Tests (6) - NEW
- `test_build_command_response_echo` - Tests C-ECHO response building
- `test_build_command_response_find_with_dataset` - Tests C-FIND response with dataset
- `test_build_move_response_with_sub_operations` - Tests C-MOVE response with sub-ops
- `test_build_get_response_with_sub_operations` - Tests C-GET response with sub-ops
- `test_build_store_response` - Tests C-STORE response building
- `test_sub_operation_counts_default` - Tests SubOperationCounts default values

#### SCP Core Tests (2)
- `test_scp_creation` - Tests SCP instantiation
- `test_default_query_provider` - Tests default query provider

### SCU Tests

#### Command Builder Tests (12) - NEW
- `test_build_command_request` - Tests DIMSE request building
- `test_build_command_request_with_dataset` - Tests request with dataset flag
- `test_parse_response_command_only` - Tests parsing command-only response
- `test_parse_response_command_with_dataset` - Tests parsing response with dataset
- `test_parse_response_command_empty_fails` - Tests empty PDU rejection
- `test_parse_response_command_data_only_fails` - Tests data-only PDU rejection
- `test_extract_status` - Tests status extraction from response
- `test_extract_status_pending` - Tests PENDING status extraction
- `test_extract_message_id_being_responded_to` - Tests message ID extraction
- `test_build_identifier_dataset` - Tests identifier dataset building
- `test_build_identifier_dataset_with_hex_tags` - Tests hex tag parsing

#### SCU Core Tests (5)
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

## SCP Integration Tests (11 tests)

Located in `tests/scp_integration.rs`:

### Core Functionality Tests (6)
- `test_scp_starts_and_stops` - SCP lifecycle management
- `test_scp_accepts_c_echo` - C-ECHO with DCMTK echoscu
- `test_scp_accepts_c_find` - C-FIND with DCMTK findscu
- `test_scp_accepts_c_store` - C-STORE with DCMTK storescu
- `test_scp_accepts_c_move` - **NEW** C-MOVE with DCMTK movescu
- `test_scp_accepts_c_get` - **NEW** C-GET with DCMTK getscu

### Error Handling Tests (5) - NEW
- `test_scp_config_validation` - Config validation rules
- `test_scp_multiple_associations` - Concurrent association handling
- `test_scp_rejects_unknown_aet` - Unknown AET handling
- `test_scp_handles_rapid_connections` - Stress testing rapid connections
- `test_scp_handles_connection_drop` - Graceful handling of dropped connections

## SCU Integration Tests (16 tests)

Located in `tests/scu_integration.rs`:

### Core Functionality Tests (5)
- `test_scu_echo_success` - C-ECHO to dcmqrscp
- `test_scu_find` - C-FIND to dcmqrscp
- `test_scu_store` - C-STORE to dcmqrscp
- `test_scu_get` - C-GET from dcmqrscp
- `test_scu_move` - C-MOVE from dcmqrscp

### Query Builder Tests (3)
- `test_scu_find_query_builder` - FindQuery builder API
- `test_scu_move_query_builder` - MoveQuery builder API
- `test_scu_get_query_builder` - GetQuery builder API

### Error Handling Tests (8) - NEW
- `test_scu_config_validation` - Config validation rules
- `test_scu_connection_timeout` - Timeout handling
- `test_scu_handles_server_disconnect` - Server disconnect handling
- `test_scu_handles_invalid_response` - Invalid port/response handling
- `test_scu_find_with_empty_results` - Empty query results handling
- `test_scu_get_with_no_results` - C-GET with no matching data
- `test_scu_validates_query_level` - Query level validation
- `test_scu_remote_node_validation_comprehensive` - Comprehensive RemoteNode validation

## Test Coverage Summary

### What's Tested
- ✅ SCP starts and stops gracefully
- ✅ SCP accepts C-ECHO, C-FIND, C-STORE from DCMTK tools
- ✅ SCP accepts C-MOVE requests (NEW)
- ✅ SCP accepts C-GET requests (NEW)
- ✅ SCP handles multiple concurrent associations
- ✅ SCP handles rapid connections/disconnections (NEW)
- ✅ SCP handles dropped connections gracefully (NEW)
- ✅ SCU configuration validation
- ✅ SCU connection timeouts
- ✅ SCU handles server disconnects (NEW)
- ✅ SCU handles empty query results (NEW)
- ✅ Query builder APIs (FIND, MOVE, GET)
- ✅ Response building for all DIMSE operations (NEW)
- ✅ PDU parsing and validation (NEW)
- ✅ Dataset metadata handling
- ✅ DICOM query level parsing

### What's NOT Tested (Future Work)
- ❌ TLS/encryption
- ❌ Query provider with real data returning results
- ❌ DICOMweb integration
- ❌ Performance/load testing
- ❌ Association negotiation edge cases
- ❌ Transfer syntax negotiation

## DCMTK Compatibility

The integration tests use DCMTK tools for end-to-end validation:
- `echoscu` - C-ECHO verification
- `findscu` - C-FIND query testing
- `storescu` - C-STORE testing
- `movescu` - C-MOVE testing (NEW)
- `getscu` - C-GET testing (NEW)
- `dcmqrscp` - Test PACS for SCU tests

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
