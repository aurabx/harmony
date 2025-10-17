# Changelog

## [0.2.0] - 2025-10-17

### Added
- Management HTTP API with dedicated routes path and network separation (AURA-2097)
- DICOMweb basic implementation, including QIDO and WADO (AURA-1963). Not yet feature complete.
- Path filter middleware (AURA-2100)
- Metadata transform (AURA-2101)
- Mock DICOM backend to test DICOMweb middleware and endpoint
- JWT test coverage

### Changed
- DICOM/DICOMweb architectural improvements and service/middleware adjustments
- Test suite reorganization

### Fixed
- Middleware registration and execution (AURA-2098)
- More robust handling of middleware error types
- jmix and jmix-rs dependency issues
- dicomweb_bridge: pass includefield as return keys to findscu (restores missing attributes)
- Various test fixes and stability improvements

### Documentation
- DICOMweb testing guide

### Dependencies
- Bumped tracing-subscriber
- Dependabot cargo updates

### Notes
- No breaking changes since 0.1.1.