# DIMSE Crate Extraction

**Last Updated**: 2026-03-14

This document describes the plan and process for extracting the `dimse` and `dicom_json_tool` crates from harmony-proxy into a standalone repository (`harmony-dimse`) so the DIMSE protocol implementation can be reused by other projects in the Runbeam ecosystem.

## Overview

The `dimse` crate implements the DICOM Message Service Element (DIMSE) protocol — SCP (inbound listener) and SCU (outbound client) — in pure Rust. It has no compile-time dependency on harmony-proxy internals and is already structured as a self-contained workspace crate. `dicom_json_tool` is a small companion crate (and CLI binary) for converting between `InMemDicomObject` and DICOM JSON Part 18; it is `dimse`'s only dev-dependency.

Both crates are moved verbatim. No public API changes are required.

## Crate Inventory

### What Moves

| Crate | Current location | Destination |
|---|---|---|
| `dimse` | `crates/dimse/` | `harmony-dimse/crates/dimse/` |
| `dicom_json_tool` | `crates/dicom_json_tool/` | `harmony-dimse/crates/dicom_json_tool/` |

### What Stays in harmony-proxy

| Path | Reason |
|---|---|
| `src/adapters/dimse/` | Integration glue: `DimseAdapter`, `PipelineQueryProvider`, `EnvelopeUnwrapper`, `StatusMapper` — all depend on harmony-proxy internals |
| `src/models/services/types/dicom*.rs` | Service type definitions (`DicomScpEndpoint`, `DicomScuBackend`, streaming variants, mock) — part of the harmony service model |
| `src/models/middleware/types/dicom*.rs` | Middleware types (`DicomToDicomwebMiddleware`, etc.) |
| `tests/dicom/` | Integration tests for the full harmony pipeline |
| `tests/dicomweb/` | Integration tests for the DICOMweb middleware bridge |
| `crates/dimse/tests/` | Moves with the `dimse` crate to `harmony-dimse` |

## Target Repository Structure

**New repository:** `runbeam-workspace/projects/harmony-dimse/`

```
harmony-dimse/
├── Cargo.toml              # Workspace root: members = ["crates/*"], resolver = "2"
├── Cargo.lock              # Not committed (library crate convention)
├── README.md
├── .gitignore              # /target, /tmp, Cargo.lock
└── crates/
    ├── dimse/              # Moved verbatim from harmony-proxy/crates/dimse/
    │   ├── Cargo.toml      # dev-dep path "../dicom_json_tool" unchanged (same relative layout)
    │   ├── src/
    │   │   ├── lib.rs
    │   │   ├── config.rs
    │   │   ├── error.rs
    │   │   ├── router.rs
    │   │   ├── types.rs
    │   │   ├── tls.rs      # feature-gated
    │   │   ├── common/
    │   │   ├── scp/
    │   │   └── scu/
    │   ├── tests/
    │   │   ├── scp_integration.rs
    │   │   ├── scu_integration.rs
    │   │   ├── scu_test_helpers.rs
    │   │   └── scp_implementation_tests.rs
    │   └── tmp/            # Created at test runtime; in .gitignore
    └── dicom_json_tool/    # Moved verbatim from harmony-proxy/crates/dicom_json_tool/
        ├── Cargo.toml      # No changes required
        └── src/
            ├── lib.rs
            ├── main.rs     # CLI binary: dicom-json
            └── model.rs
```

The workspace `Cargo.toml` at the root contains only:

```toml
[workspace]
members = ["crates/*"]
resolver = "2"
```

No version bumps are needed at extraction time. Both crates stay at `0.12.0` until the first versioned external release.

## Dependency Reference Strategy

harmony-proxy references `harmony-dimse` differently at each stage of development:

### Stage 1 — Local development (before a GitHub remote exists)

Use relative path references. Both repos must be checked out as siblings:

```toml
# harmony-proxy/Cargo.toml
dimse = { path = "../harmony-dimse/crates/dimse" }
dicom_json_tool = { path = "../harmony-dimse/crates/dicom_json_tool" }
```

### Stage 2 — After pushing harmony-dimse to GitHub

Switch to a git + branch reference:

```toml
# harmony-proxy/Cargo.toml
dimse = { git = "ssh://git@github.com/runbeam/harmony-dimse", branch = "main" }
dicom_json_tool = { git = "ssh://git@github.com/runbeam/harmony-dimse", branch = "main" }
```

To pin to a specific commit during development:

```toml
dimse = { git = "ssh://git@github.com/runbeam/harmony-dimse", rev = "<commit-sha>" }
```

### Stage 3 — After publishing to crates.io

```toml
dimse = "0.12"
dicom_json_tool = "0.12"
```

## Migration Steps

### Phase 1 — Create the new repository

1. Create the directory structure:
   ```bash
   mkdir -p runbeam-workspace/projects/harmony-dimse/crates
   cd runbeam-workspace/projects/harmony-dimse
   git init
   ```

2. Write the workspace `Cargo.toml` (members only, no package section):
   ```toml
   [workspace]
   members = ["crates/*"]
   resolver = "2"
   ```

3. Write `.gitignore`:
   ```
   /target
   /tmp
   Cargo.lock
   ```

### Phase 2 — Copy the crates (copy first, delete after verification)

4. Copy `crates/dimse/` into `harmony-dimse/crates/dimse/`:
   ```bash
   cp -r harmony-proxy/crates/dimse harmony-dimse/crates/dimse
   ```

5. Copy `crates/dicom_json_tool/` into `harmony-dimse/crates/dicom_json_tool/`:
   ```bash
   cp -r harmony-proxy/crates/dicom_json_tool harmony-dimse/crates/dicom_json_tool
   ```

6. No `Cargo.toml` changes are required in either crate. The `dimse` dev-dependency path `../dicom_json_tool` resolves identically in the new layout. The `dimse` crate has zero compile-time references to harmony-proxy types.

### Phase 3 — Verify in isolation

7. Build the new workspace:
   ```bash
   cd harmony-dimse
   cargo build
   ```

8. Run the test suite:
   ```bash
   cargo test
   ```

   All tests in `crates/dimse/tests/` should pass. See [Test Data and External Dependencies](#test-data-and-external-dependencies) for prerequisites.

### Phase 4 — Update harmony-proxy

9. In harmony-proxy's root `Cargo.toml`, replace the path dependencies:

   ```toml
   # Remove:
   dimse = { path = "crates/dimse" }
   dicom_json_tool = { path = "crates/dicom_json_tool" }

   # Add (Stage 1 — local path):
   dimse = { path = "../harmony-dimse/crates/dimse" }
   dicom_json_tool = { path = "../harmony-dimse/crates/dicom_json_tool" }
   ```

   Update the `[dev-dependencies]` section identically (harmony-proxy references `dimse` there too).

10. The workspace `members = ["crates/*"]` glob will automatically exclude the removed directories once they are deleted. No explicit change to the workspace members list is needed.

11. Delete the crates from harmony-proxy:
    ```bash
    rm -rf harmony-proxy/crates/dimse
    rm -rf harmony-proxy/crates/dicom_json_tool
    ```

12. Verify harmony-proxy still compiles:
    ```bash
    cd harmony-proxy
    cargo build
    ```

13. Verify the full test suite still passes:
    ```bash
    cargo test
    ```

### Phase 5 — Housekeeping

14. Write a `README.md` for `harmony-dimse` describing the two crates, their purpose, and basic usage.

15. Make an initial commit and tag:
    ```bash
    cd harmony-dimse
    git add .
    git commit -m "feat: initial extraction from harmony-proxy v0.12.0"
    git tag v0.12.0
    ```

16. Push to GitHub and update harmony-proxy's Cargo reference from local path to git URL (see [Stage 2](#stage-2--after-pushing-harmony-dimse-to-github)).

## Test Data and External Dependencies

The `crates/dimse/tests/` suite is fully hermetic with respect to data files.

### No fixture files required

All DICOM datasets are constructed in-process:

- `scp_integration.rs` builds `InMemDicomObject` instances directly via the `dicom-object` crate and writes them to `tempfile::tempdir()` locations.
- `scu_integration.rs` uses a `create_test_dicom_file()` helper that constructs and serialises a minimal DICOM object at runtime.
- No pre-existing `.dcm`, `.json`, or fixture files are read from disk.
- No `samples/` directory references exist in the `dimse` crate tests.

### Runtime temp directories

Tests write to `../../tmp/qrscp_<name>_<uuid>/` relative to `crates/dimse/`. When running `cargo test` from the crate root, this resolves to `harmony-dimse/tmp/` — the same `./tmp` convention used by harmony-proxy. Add `tmp/` to `.gitignore`.

The `storage_dir` field in several tests is set to `/tmp/dimse_test` or `/tmp/dimse_test_move` (system temp); these are created by the SCP at runtime and do not need to exist beforehand.

### DCMTK CLI tools

The SCU integration tests (`scu_integration.rs`) spawn DCMTK processes. These tests require the following tools to be on `PATH`:

| Tool | Used for |
|---|---|
| `dcmqrscp` | QR SCP server for C-FIND / C-GET / C-MOVE tests |
| `dcmqridx` | Indexing DICOM files into the QR SCP database |
| `storescu` | Sending C-STORE to the QR SCP |
| `getscu` | C-GET operations |

Tests are guarded by `dcmtk_available()` — if any tool is missing, the test is skipped rather than failing.

Install DCMTK:
```bash
# macOS
brew install dcmtk

# Debian / Ubuntu
sudo apt-get install dcmtk
```

### Environment variables

| Variable | Effect |
|---|---|
| `HARMONY_TEST_VERBOSE_DCMTK=1` | Passes `-d` to `dcmqrscp` and shows child process stdout/stderr. Off by default. |

## What Does Not Change

The public API of the `dimse` crate is unchanged by the extraction:

| Item | Notes |
|---|---|
| `QueryProvider` trait | The plugin point for SCP behaviour. harmony-proxy implements this via `PipelineQueryProvider` in `src/adapters/dimse/query_provider.rs`, which stays in harmony-proxy. |
| `DimseScp` | SCP entry point — unchanged. |
| `DimseScu` / `ScuBuilder` | SCU client — unchanged. |
| All types (`DatasetStream`, `DimseConfig`, `RemoteNode`, `FindQuery`, `MoveQuery`, `GetQuery`, `QueryLevel`, `DimseStatus`, `DimseError`) | Unchanged. |
| `InMemoryRouter` / `Router` trait | Unchanged. |
| TLS feature flag | `features = ["tls"]` — unchanged. |

`PipelineQueryProvider` (the bridge between DIMSE operations and the harmony pipeline executor) remains in `src/adapters/dimse/query_provider.rs` inside harmony-proxy. It is not part of the `dimse` crate and is not moving.

## Known Risks

| Risk | Mitigation |
|---|---|
| `Cargo.lock` version drift between repos | Libraries do not commit `Cargo.lock`. harmony-proxy's lock file governs resolved versions. Use `cargo update --precise <crate> --version <ver>` to pin specific versions when needed. |
| Git dependency resolution in CI | Add `.cargo/config.toml` with `[net] git-fetch-with-cli = true` to the harmony-proxy repo if CI struggles with SSH git deps. |
| `dicom_json_tool` CLI binary must remain independently buildable | The workspace layout preserves this — `cargo install --path crates/dicom_json_tool` works from the `harmony-dimse` root. |
| `../../tmp` path assumption breaks if invocation directory changes | `cargo test` always sets the working directory to the crate root (`crates/dimse/`). This is standard Cargo behaviour and is unaffected by where the repo lives on disk. |
