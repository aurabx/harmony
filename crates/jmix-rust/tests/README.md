# JMIX-RS Test Suite

This directory contains comprehensive tests for the jmix-rs library.

## Test Structure

### Unit Tests (in `src/`)
- Each module includes unit tests for its specific functionality
- Run with: `cargo test --lib`

### Integration Tests (in `tests/`)
- `integration_tests.rs` - Full integration tests with realistic data
- `sample_validation_tests.rs` - Tests using sample JSON files in `/samples`

### Sample Data (in `/samples`)
- `sample_config.json` - Example configuration file
- `sample_manifest.json` - Example JMIX manifest
- `sample_metadata.json` - Example JMIX metadata
- `sample_audit.json` - Example JMIX audit trail
- `sample_files.json` - Example files manifest

## Running Tests

### All Tests
```bash
cargo test
```

### Unit Tests Only
```bash
cargo test --lib
```

### Integration Tests Only
```bash
cargo test --test integration_tests
cargo test --test sample_validation_tests
```

### Specific Test Module
```bash
cargo test types::tests
cargo test validation::tests
cargo test dicom::tests
```

### Schema Validation Tests
These tests require the JMIX schema files to be present in `../jmix/schemas/`:
```bash
# Run ignored tests (schema validation)
cargo test -- --ignored
```

### With Output
```bash
cargo test -- --nocapture
```

## Test Categories

### 🧪 **Type System Tests**
- Serialization/deserialization roundtrips
- Contact variant handling
- UUID and hash format validation
- Complete envelope structure testing

### 📋 **Configuration Tests**
- Sample config file loading
- Contact type parsing (email vs detailed)
- Config to JMIX type conversion

### ✅ **Validation Tests**
- Schema validation (when schema files are present)
- Structure validation without schemas
- Error handling for missing schemas

### 🏥 **DICOM Processing Tests**
- DICOM file detection by magic number and extension
- DICOM date and name format conversion
- Fallback to configuration when DICOM parsing fails
- Empty directory handling

### 📁 **Sample File Tests**
- Loading and parsing all sample JSON files
- Consistency checks across sample data
- Round-trip conversion tests

## Expected Test Output

When all tests pass, you should see output similar to:
```
running 25 tests
test types::tests::test_envelope_serialization_roundtrip ... ok
test types::tests::test_contact_variants ... ok
test validation::tests::test_validator_creation ... ok
test dicom::tests::test_dicom_processor_creation ... ok
test dicom::tests::test_format_dicom_date ... ok
test dicom::tests::test_format_dicom_person_name ... ok
[... more tests ...]

test result: ok. 25 passed; 0 failed; 2 ignored; 0 measured
```

## Ignored Tests

Some tests are ignored by default because they require external dependencies:

- **Schema validation tests** - Require `../jmix/schemas/` directory with JSON schema files
- **Real DICOM processing tests** - Would require actual DICOM files for full testing

To run ignored tests:
```bash
cargo test -- --ignored
```

## Adding New Tests

When adding new functionality:

1. **Add unit tests** in the same file as your implementation (in `src/`)
2. **Add integration tests** in `tests/integration_tests.rs` 
3. **Add sample data** in `/samples` if testing new data structures
4. **Update this README** if you add new test categories

## Troubleshooting

### Missing Sample Files
If you see "Skipping test - sample file not found", the sample JSON files may not be present. Check that files exist in the `/samples` directory.

### Schema Validation Failures
If schema validation tests fail, ensure:
1. The `../jmix/schemas/` directory exists
2. Schema files are present: `manifest.schema.json`, `metadata.schema.json`, `audit.schema.json`, `files.schema.json`
3. Schema files are valid JSON Schema format

### Compilation Errors
Make sure all dependencies are installed:
```bash
cargo check
```

If you see missing dependencies, run:
```bash
cargo build
```