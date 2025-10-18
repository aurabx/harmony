# JMIX API Development Testing

Quick guide for testing the JMIX API during development.

## Quick Start

Start the proxy with JMIX configuration:

```bash
cargo run -- --config examples/jmix-to-dicom/config.toml
```

**Prerequisites:**
- Rust toolchain
- DICOM samples in `./samples/` (optional, for testing)
- Orthanc at `127.0.0.1:4242` (or DCMTK - see below)

**Configuration notes:**
- HTTP server: `127.0.0.1:8080` 
- JMIX prefix: `/jmix`
- DICOM backend: Orthanc at `127.0.0.1:4242` (AET: `ORTHANC`)

### Alternative: DCMTK Setup

If you don't have Orthanc:

```bash
# Install DCMTK
brew install dcmtk                    # macOS
sudo apt-get install -y dcmtk        # Debian/Ubuntu

# Edit examples/jmix-to-dicom/pipelines/jmix_to_dicom.toml
# Change host/port/aet to match your DCMTK setup
```

## API Routes

All routes assume `127.0.0.1:8080/jmix`:

### Build JMIX from StudyInstanceUID (triggers DICOM retrieval)
```zsh
curl -fsSLOJ -H "Accept: application/zip" \
  "http://127.0.0.1:8080/jmix/api/jmix?studyInstanceUid=1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0"
```

### Get manifest by ID
```zsh
curl -fsSL -H "Accept: application/json" \
  "http://127.0.0.1:8080/jmix/api/jmix/<id>/manifest" \
  -o <id>.manifest.json
```

### Download archive by ID
```zsh
curl -fsSLOJ -H "Accept: application/zip" \
  "http://127.0.0.1:8080/jmix/api/jmix/<id>"
```

### Upload JMIX envelope
```zsh
curl -fsS -X POST -H "Content-Type: application/zip" \
  --data-binary @<file>.zip \
  "http://127.0.0.1:8080/jmix/api/jmix" -D jmix.post.headers.txt -o jmix.post.json
```

**Status codes:** 200/201 (success), 404 (not found), 415 (wrong content-type), 409 (exists), 400 (validation error)

## Storage & Paths

**JMIX store layout:**
- `./tmp/jmix-store/<id>/manifest.json`
- `./tmp/jmix-store/<id>/payload/...` 
- `./tmp/jmix-store/<id>.zip`
- `./tmp/jmix-store/jmix-index.redb` (search index)

**Configuration options:**
- `path_prefix` - API path (default: `/jmix`)
- `skip_hashing` - Skip SHA256 for performance (default: `false`)
- `skip_listing` - Skip file listings (default: `false`) 
- `store_dir` - Storage directory (default: `./tmp/jmix-store`)

Query parameters can override config defaults: `?skip_hashing=true&skip_listing=true`

## Troubleshooting

**Enable detailed logs:**
```bash
RUST_LOG=harmony=debug cargo run -- --config examples/jmix-to-dicom/config.toml
```

**DIMSE debugging:**
```bash
HARMONY_TEST_VERBOSE_DCMTK=1 RUST_LOG=harmony=debug cargo run -- --config examples/jmix-to-dicom/config.toml
```

**Common issues:**
- 404: No data for StudyInstanceUID in PACS
- 415: Wrong Content-Type header
- 409: JMIX ID already exists
- 400: Schema validation failed

**Note:** GET by ID/manifest serves from local store; DICOM backend not required.

## Code References

- Endpoint: `src/models/services/types/jmix.rs`
- Builder middleware: `src/models/middleware/types/jmix_builder.rs`
- Index: `src/models/middleware/types/jmix_index.rs`
- Tests: `tests/jmix/`