# DICOM SCU/SCP Separation Integration Tests

## Overview

Comprehensive integration tests for the DICOM SCU (Service Class User) and SCP (Service Class Provider) service separation implemented in Harmony proxy.

## Test Files

### 1. `scu_scp_separation.rs` - Integration Tests

Full integration tests that validate the SCU/SCP separation using actual DCMTK tools.

**Tests:**

1. **test_dicom_scu_backend_cfind** ✅
   - Tests `dicom_scu` service as a backend
   - Spawns `dcmqrscp` as external PACS
   - Routes HTTP -> SCU backend -> DCMTK PACS
   - Validates C-FIND operation works correctly

2. **test_dicom_scp_endpoint_cfind** ⏭️ (ignored)
   - Tests `dicom_scp` service as an endpoint
   - Requires full service running with SCP listener
   - Uses `findscu` to query Harmony SCP endpoint
   - **Reason for ignore:** Requires DIMSE adapter SCP listener to be started

3. **test_scu_cannot_be_endpoint** ✅
   - Validates that `dicom_scu` cannot be used as an endpoint
   - Configuration validation should fail with clear error message

4. **test_scp_cannot_be_backend** ✅
   - Validates that `dicom_scp` cannot be used as a backend
   - Configuration validation should fail with clear error message

5. **test_legacy_dicom_service_backward_compat** ✅
   - Tests backward compatibility with legacy "dicom" service name
   - Verifies "dicom" maps to "dicom_scu" for backends
   - Ensures existing configurations continue to work

6. **test_pipeline_scp_receives_external_find** ⏭️ (ignored)
   - Tests SCP endpoint receiving requests from external SCU
   - Tests C-ECHO and C-FIND operations
   - **Reason for ignore:** Requires DIMSE adapter SCP listener to be started

7. **test_full_pipeline_http_to_scu_to_pacs** ✅
   - Complete end-to-end pipeline test
   - HTTP endpoint -> SCU backend -> External PACS
   - Validates full request/response cycle

### 2. `dicom_scu_scp_config_validation.rs` - Config Validation Tests

Fast unit-style tests for configuration validation without requiring DCMTK.

**Tests:**

1. **test_dicom_scu_backend_validation_success** ✅
   - Valid `dicom_scu` backend configuration
   
2. **test_dicom_scu_missing_remote_aet** ✅
   - Backend with missing AET (runtime validation)
   
3. **test_dicom_scu_missing_host** ✅
   - Backend with missing host (runtime validation)
   
4. **test_dicom_scp_endpoint_validation_success** ✅
   - Valid `dicom_scp` endpoint configuration
   
5. **test_dicom_scp_invalid_aet_empty** ✅
   - SCP with empty AET should fail validation
   
6. **test_dicom_scp_invalid_aet_too_long** ✅
   - SCP with AET > 16 chars should fail validation
   
7. **test_dicom_scp_no_operations_enabled** ✅
   - SCP with no operations enabled should fail
   
8. **test_dicom_scp_with_c_get_enabled** ✅
   - Valid SCP configuration with C-GET enabled
   
9. **test_complete_scp_to_scu_bridge** ✅
   - Complete bridge configuration: SCP endpoint -> SCU backend

## Test Results

```
dicom_scu_scp_separation:        5 passed, 0 failed, 2 ignored
dicom_scu_scp_config_validation: 9 passed, 0 failed
```

## Implementation Changes

### 1. Service Registry
- Added backward compatibility mapping: `"dicom"` -> `"dicom_scu"`
- Located in: `src/models/services/services.rs`

### 2. Configuration Validation
- Added endpoint validation to reject `dicom_scu` and legacy `"dicom"` as endpoints
- Added backend validation to reject `dicom_scp` as backends
- Located in: `src/config/config.rs`

### 3. Test Registration
- Added tests to `Cargo.toml`:
  - `dicom_scu_scp_separation` (integration tests)
  - `dicom_scu_scp_config_validation` (config tests)

## Running Tests

### Run All Tests
```bash
cargo test --test dicom_scu_scp_separation
cargo test --test dicom_scu_scp_config_validation
```

### Run Specific Test
```bash
cargo test --test dicom_scu_scp_separation test_dicom_scu_backend_cfind
```

### Run Including Ignored Tests
```bash
cargo test --test dicom_scu_scp_separation -- --ignored
```

Note: Ignored tests require manual setup of SCP listeners and may not work in the current test harness.

### Enable DCMTK Verbose Output
```bash
HARMONY_TEST_VERBOSE_DCMTK=1 cargo test --test dicom_scu_scp_separation
```

## Test Dependencies

### Required for Integration Tests
- DCMTK tools (`dcmqrscp`, `storescu`, `findscu`, `echoscu`)
- Tests gracefully skip if DCMTK is not available

### Required for Config Tests
- No external dependencies

## Coverage Summary

✅ **Covered:**
- SCU backend functionality with outgoing DICOM operations
- Configuration validation for service type restrictions
- Backward compatibility with legacy "dicom" service name
- Pipeline integration (HTTP -> SCU -> PACS)
- Config validation for all service parameters

⏭️ **Deferred (Ignored Tests):**
- SCP endpoint with incoming DICOM requests
- SCP listener lifecycle management
- External SCU to Harmony SCP communication

**Reason:** SCP tests require the DIMSE adapter's SCP listener to be started, which is not currently supported in the test harness. These tests are marked as `#[ignore]` and documented for future implementation.

## Future Improvements

1. **SCP Listener Test Harness**
   - Implement test infrastructure to start DIMSE adapter SCP listeners
   - Enable the currently ignored SCP endpoint tests

2. **Additional Operations**
   - Test C-STORE for SCP endpoints
   - Test C-MOVE operations in both directions
   - Test C-GET with actual data retrieval

3. **Error Cases**
   - Test connection failures
   - Test invalid DICOM messages
   - Test timeout scenarios

4. **Performance Tests**
   - Concurrent request handling
   - Large dataset transfers
   - Load testing for SCP listeners
