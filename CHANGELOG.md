# Changelog

## [0.3.1] - 2025-10-21

### Added
- Improved ability to set TargetDetails in middleware
- (AURA-2103) HTTP backend with tests and related fixes
- (AURA-2113) Docker build support

### Fixed
- DIMSE op selection in DICOM backend
- Correctly extract manifest from zip if not available
- Echo endpoint functionality and added extra tracing
- Moved dimse_retrieve_mode to DICOM backend configuration
- Moved jmix options to the jmix_builder

## [0.3.0] - 2025-10-19

### Highlights
- Complete protocol adapter redesign built around the Router/Pipeline architecture for clearer request flow and easier extensibility. See docs/router.md and docs/adapters.md.
- Enhanced DIMSE support with more robust association handling and improved behavior for common operations.
- Configurable DICOM backends via a new backend abstraction, allowing you to select and configure different DICOM providers.
- Examples reorganized to match the new adapter/backends layout and to simplify getting started.
- Automatic persistent SCP orchestration for DICOM backend operations.
- Numerous bug fixes and dependency updates.

### Breaking Changes
- Protocol adapter interfaces were redesigned.
- Configuration format related to protocol adapters/backends has changed.

### Added
- Protocol-agnostic core module layout with HTTP and DIMSE adapters
- Backend abstraction for DICOM enabling configurable providers
- Automatic persistent DICOM SCP spawning for backend operations
- Configurable GET/MOVE operations for DICOM backends
- ServiceType protocol-agnostic response hooks
- Comprehensive tests for Phase 1 components

### Changed
- Router and pipeline integration updated to align with the new adapter design
- Examples directory layout updated with focused example directories
- DIMSE response conversion and status mapping improvements
- Deprecated dispatcher in favor of HttpAdapter delegation
- Management API fixes and improvements

### Fixed
- Backend skipping logic and associated tests
- State issues with JMIX package construction
- Management API functionality
- Transform test references updated to new example paths
- Multiple stability and correctness fixes across adapters, routing, and configuration validation

### Dependencies
- Updated jmix-rs to 0.3.2
- Upgraded core web stack and ecosystem crates

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

[0.3.1]: https://github.com/aurabx/harmony/compare/0.3.0...0.3.1
[0.3.0]: https://github.com/aurabx/harmony/compare/0.2.0...0.3.0
[0.2.0]: https://github.com/aurabx/harmony/compare/0.1.1...0.2.0
