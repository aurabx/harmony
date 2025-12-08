# Changelog

## [Unreleased]

### Added
- Environment variable validation: `proxy.required_env_vars` field enforces required environment variables at startup with clear error messages
- Sensitive field patterns: `proxy.sensitive_field_patterns` field for log redaction rules

### Changed
- JWT authentication: Added support for deprecated `leeway_seconds` field as fallback to `leeway_secs` for backward compatibility

## [0.9.0] - 2025-12-07

### Highlights
- Enhanced debugging and data transformation capabilities with new middleware
- Improved DICOM handling with structure flattening/unflattening
- Extended DIMSE SCP service with C-STORE support
- Configuration flexibility with environment variable replacement
- Refactored and streamlined DIMSE implementation

### Added
- (AURA-2226) Data dumper middleware for inspecting and debugging pipeline data
- New `dicom_flatten` and `dicom_unflatten` middleware for flattening DICOM structures in pipelines
- Additional `left` / `right` transform application options in middleware configuration
- Filesystem/S3 backend type
- Support C-STORE on the DIMSE SCP service
- Support for environment variable replacement in configuration files

### Changed
- Transform pipeline now uses a more consistent internal data structure for normalized/transform data
- DIMSE crates (SCU/SCP) refactored to reduce reliance on dcmtk and improve maintainability
- Demo scripts updated to reflect current examples and configuration
- Minor internal cleanups

### Fixed
- Transform behavior and FHIR DICOM example pipeline reliability improvements
- FHIR and FHIR-DICOM example authentication and configuration issues
- DIMSE task handling and related example correctness
- Doctest stability and test-only file handling
- Improved configuration error handling
- Config reload blocking on first run

### Documentation
- General docs improvements and pass over existing content
- Chore-level documentation updates and AI assistant rules refinements

### Dependencies

## [0.8.0] - 2025-11-24

### Highlights
- Major authentication and authorization overhaul with new authentication handling system
- Normalized connection configuration support for all endpoint and backend types
- Updated to runbeam-sdk 0.9 for improved cloud integration
- Enhanced peer and target configuration with connection references

### Breaking Changes
- Authentication system has been redesigned - old auth configurations need to be migrated to the new authentication handling system
- Backend and target configurations now support authorization settings that may require config updates

### Added
- (AURA-2196) New authorization support for backends and targets
  - Fine-grained authorization controls at the backend level
  - Target-specific authorization configuration
  - Integration with new authentication handling system
- (AURA-2196) Redesigned authentication handling system
  - More flexible and extensible authentication framework
  - Improved JWT token validation
  - Better separation of authentication and authorization concerns
- (AURA-2205) Normalized ConnectionConfig support for all services
  - `fhir`, `dicomweb`, `jmix`, and `echo` endpoints now support connection settings
  - Backend services respect connection configuration including `base_path` and protocol-derived URLs
  - Configuration references via `target_ref` and `peer_ref` for cleaner configuration
- (AURA-1973, AURA-2127) Basic peer and target configuration
  - Foundation for peer-to-peer communication patterns
  - Target configuration for more flexible routing

### Changed
- Replaced existing authentication system with new authentication handling framework
- Enhanced configuration validation for connection settings
- Improved error messages for authentication and authorization failures

### Fixed
- JWT token validation error that could cause authentication failures
- Connection configuration handling for various service types

### Dependencies
- Updated runbeam-sdk to 0.9.0 (from 0.7.x)
- SDK update to 0.8 as intermediate step

## [0.7.1] - 2025-11-20

### Highlights
- Enhanced deployment options with Helm charts and Docker Compose
- Improved security and deployment best practices
- Better documentation for cloud deployment scenarios

### Added
- Helm chart for Kubernetes deployment with security best practices
- Docker Compose quickstart for simple installation
- Cloud Run deployment documentation
- Pre-built Docker images for easier deployment

### Changed
- Updated Docker base image to debian:stable-slim for improved security
- Removed version pinning from docker-compose for flexibility
- Improved Helm chart security (container UID collision prevention, non-root user)

### Fixed
- Container UID collision prevention in Helm charts
- Security hardening in Helm deployment configuration

## [0.7.0] - 2025-11-19

### Highlights
- Updated runbeam-sdk integration to 0.7.x for improved cloud connectivity
- Removed OpenSSL dependencies from build pipeline for better portability
- Enhanced configuration handling for Runbeam authorization and API defaults
- Docker build improvements and testing infrastructure

### Changed
- Updated runbeam-sdk dependency to 0.7.x
- Improved config handling for cloud integration
- Enhanced Docker build workflow with better testability
- Standardized internal crate versions to 0.7.0 across workspace

### Fixed
- Runbeam authorization and config sync issues
- Default API URL configuration in cloud integration
- OpenSSL dependencies removed from build pipeline for improved cross-platform support
- Docker build dependencies (libdus) for production images
- Test stability and reliability improvements

### Dependencies
- Updated runbeam-sdk to 0.7.x
- Internal workspace crates (harmony-database, harmony-filesystem) bumped to 0.7.0

## [0.6.0] - 2025-11-17

### Highlights
- Binary name standardized to `harmony` for a consistent CLI and deployment experience
- New policy-based middleware for fine-grained request handling and authorization
- Additional data type handling in core pipeline processing (AURA-2195)
- New smoketest example to validate end‑to‑end configuration quickly

### Breaking Changes
- Binary name is now `harmony` (previously some docs referred to `harmony-proxy`); update any custom scripts or automation accordingly

### Added
- Policy-based middleware with new policy types and examples
- Additional content/data type support in the pipeline (AURA-2195)
- New `smoketest` example configuration to exercise key paths
- Internal crate versions bumped to 0.6.0 (`dimse`, `dicom_json_tool`, `harmony_transform`) for consistency with the main `harmony` crate

### Changed
- Standardized documentation examples to use the `harmony` binary name
- Clarified install and Docker usage instructions around the `harmony` binary
- Updated examples to reflect current policy and pipeline behavior
- Renamed release build workflow for clearer CI semantics

### Fixed
- Hot reload now correctly watches and applies updates to pipelines
- Fixed config watching failures for pipeline updates
- Query/path parameter handling no longer strips parameters from paths
- Hop-by-hop headers are no longer proxied to backends
- Policy configuration and return types aligned with the new middleware
- Minor path and build warnings, plus various test and example fixes

### Documentation
- Updated README and docs to reference `harmony` as the primary binary name
- Added/expanded policy and installation documentation

### Dependencies
- Internal crates updated to version 0.6.0 (no external dependency version changes)
- runbeam-sdk dependency refreshed to the latest compatible 0.6.x

## [0.5.0] - 2025-11-10

### Highlights
- Hot configuration reload with zero-downtime updates for routes/middleware/backends
- Runbeam Cloud integration for gateway authentication and authorization
- Production-ready DICOM SCP implementation with full C-ECHO, C-FIND, C-GET, C-MOVE support
- Enhanced security with encryption key management and token storage
- Migration from fulvio-jolt to jolt-rs for transform middleware

### Breaking Changes
- Transform middleware now requires jolt-rs instead of fluvio-jolt
- JOLT specification format remains compatible but dependency has changed

### Added
- (AURA-2175) Hot-swappable configuration with file watcher and zero-downtime reloads
  - Automatic config reload on file changes (200ms debounce)
  - Zero-downtime updates for middleware, routes, backends, logging, and storage config
  - Selective adapter restart for network topology changes
  - Config validation before applying changes with automatic rollback on errors
- (AURA-2064) Runbeam Cloud authentication and authorization integration
  - JWT-based gateway authorization via Management API `/authorize` endpoint
  - Machine token lifecycle management with 30-day expiry
  - Secure token storage with OS keyring support (macOS Keychain, Linux Secret Service, Windows Credential Manager)
  - Encrypted filesystem storage fallback using age X25519 encryption
  - Cloud polling for configuration and control plane synchronization
- (AURA-2073) Production-ready DICOM SCP implementation
  - Complete C-ECHO support for connectivity testing
  - C-FIND query operations with patient/study/series/image levels
  - C-MOVE dataset transfer requests
  - C-GET direct dataset retrieval
  - Configurable AE titles and service endpoints
  - Automatic persistent SCP orchestration for backend operations
- (AURA-2156) Integration with runbeam-sdk for unified API access
  - Replaced custom Runbeam API implementation with SDK
  - Unified storage abstractions for tokens and credentials
  - Better error handling and retry logic
- Security and encryption key management
  - Environment variable support for RUNBEAM_ENCRYPTION_KEY and RUNBEAM_JWT_SECRET
  - Multiple key management strategies (CLI-managed, environment variables, auto-generated)
  - Comprehensive encryption key documentation and best practices
  - Production deployment examples for Docker, Kubernetes, AWS ECS
- Network-aware unified startup and configuration
  - Adapter registry for managing protocol adapters lifecycle
  - Network-specific adapter spawning based on pipeline requirements
  - Improved startup logging and diagnostics
- Enhanced integration testing infrastructure
  - Easier test execution with improved test organization
  - Config reload integration tests
  - Cloud poller integration tests
  - Adapter registry lifecycle tests

- Auto-generated management network configuration
  - Management network automatically created if enabled but not explicitly configured
  - Simplifies setup for management API functionality
- Enhanced cloud polling with transform auto-download
  - Automatic download of referenced transform specifications before applying cloud configs
  - Improved error handling and validation during config sync
- Path filter middleware TOML serialization fixes

### Changed
- Replaced fulvio-jolt dependency with jolt-rs for transform middleware
  - Maintains compatibility with existing JOLT specifications
  - Improved performance and reliability
- Updated runbeam-sdk integration to latest version
  - Removed custom Runbeam API implementations
  - Simplified token management code
- Reorganized helper functions and test utilities
- Improved adapter spawning and lifecycle management

### Fixed
- SCP service initialization and lifecycle bugs
- Services no longer start if no endpoints are defined (prevents resource waste)
- C-ECHO and C-FIND test validation in DICOM SCP examples
- Workflow issues causing basic Rust test failures
- Various test stability improvements

### Documentation
- Added comprehensive encryption key management guide (docs/encryption-key-management.md)
- Enhanced security documentation with environment variables and deployment examples (docs/security.md)
- Added hot config reload architecture documentation (docs/config-reload.md)
- Updated transform middleware documentation for harmony-jolt migration

### Dependencies
- Added harmony-jolt 0.5.0 (replacing fluvio-jolt)
- Updated runbeam-sdk integration
- Added notify 6.1 for file watching
- Added arc-swap 1.7 for atomic config swapping

### Notes
- BREAKING: Transform middleware requires harmony-jolt dependency update
- Hot reload feature is production-ready as of this release
- DICOM SCP implementation is now feature-complete for Phase 1 requirements
- Runbeam Cloud integration requires environment variable configuration (RUNBEAM_JWT_SECRET)

## [0.4.0] - 2025-10-24

### Added
- FHIR backend support (AURA-2102)
- DICOMweb pagination, offset, filters, and date range queries (AURA-2087, AURA-2088)
- Config validation for TOML files
- End-to-end testing scripts

### Fixed
- JMIX builder wrong response with wrong header
- Incorrect slashes in paths
- mock_dicom backend patient level queries
- Transforms loading from non-app-root
- Missing middleware in validation
- Examples and test configurations

### Changed
- Refactored adaptor spawning logic

## [0.3.2] - 2025-10-22

### Added
- (AURA-2086) FHIR to DICOM pipeline support
- Unified CI/CD workflows for builds and releases
- Helm chart for Kubernetes deployment

### Changed
- Updated metadata transform functionality
- Unified Docker build workflows
- DockerHub repository naming

### Fixed
- FHIR to DICOM conversion
- Windows build shasum dependency
- OpenSSL dependency handling in builds
- Docker image tagging for releases

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

[0.9.0]: https://github.com/aurabx/harmony/compare/0.8.0...0.9.0
[0.8.0]: https://github.com/aurabx/harmony/compare/0.7.1...0.8.0
[0.7.1]: https://github.com/aurabx/harmony/compare/0.7.0...0.7.1
[0.7.0]: https://github.com/aurabx/harmony/compare/0.6.0...0.7.0
[0.6.0]: https://github.com/aurabx/harmony/compare/0.5.0...0.6.0
[0.5.0]: https://github.com/aurabx/harmony/compare/0.4.0...0.5.0
[0.4.0]: https://github.com/aurabx/harmony/compare/0.3.4...0.4.0
[0.3.2]: https://github.com/aurabx/harmony/compare/0.3.1...0.3.2
[0.3.1]: https://github.com/aurabx/harmony/compare/0.3.0...0.3.1
[0.3.0]: https://github.com/aurabx/harmony/compare/0.2.0...0.3.0
[0.2.0]: https://github.com/aurabx/harmony/compare/0.1.1...0.2.0
