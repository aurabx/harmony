# dicom_to_dicomweb Bridge – Current Behavior and Gaps

_Last reviewed: 2025-11-30_

This note documents the current `dicom_to_dicomweb` middleware behavior and the main issues/gaps in the DIMSE→DICOMweb bridge.

## 1. High-level Flow

Example pipeline: `examples/dicom_to_dicomweb/pipelines/bridge.toml`.

- Endpoint: `dicom_listener` (`service = "dicom_scp"`)
  - DIMSE SCP via `DimseAdapter`.
  - Uses `adapters::dimse::query_provider::PipelineQueryProvider`.
- Middleware: `dicom_to_dicomweb` (`src/models/middleware/types/dicom_to_dicomweb.rs`)
  - **LEFT** (request): DIMSE → DICOMweb HTTP.
    - C-FIND → QIDO-RS `GET /studies` with tag-based query params.
    - C-STORE → STOW-RS `POST /studies` with `multipart/related; type="application/dicom"` body built from a local file.
    - C-GET / C-MOVE → WADO-RS `GET` on `/studies/.../series/.../instances/...` or parent levels, with `Accept: multipart/related; type="application/dicom"`.
  - **RIGHT** (response): HTTP → DIMSE-ish metadata.
    - Sets `response_details.metadata["dicom_status"]` based on HTTP status.
    - For C-FIND: populates `normalized_data` as array + `result_count`.
    - For C-GET/C-MOVE: sets `response_format`, `multipart_boundary`, `dataset_count` etc. based on `Content-Type`.
- Backend: `service = "http"` to DICOMweb (e.g. Orthanc) via `HttpEndpoint`.

## 2. Critical Functional Issues

### 2.1 C-GET / C-MOVE WADO bodies are lost in the pipeline

Path: HTTP backend → outgoing middleware → DIMSE layer.

- `PipelineExecutor::process_outgoing_middleware` always converts `ResponseEnvelope<Vec<u8>>` to `ResponseEnvelope<Value>` using `to_json()`.
  - For non-JSON content types (`multipart/related`, `application/dicom`), `is_json == false`.
  - `normalized_data` remains `None` and `original_data` becomes `Value::Null`.
- `dicom_to_dicomweb.right` operates on `ResponseEnvelope<Value>`.
  - It can see `response_details.status` and `response_details.headers` (for content-type, boundary), and sets metadata like `dicom_status`, `response_format`, `multipart_boundary`.
  - It has **no access** to the raw WADO bytes anymore.
- After middleware, `ResponseEnvelope<Value>::to_bytes()` runs:
  - `normalized_data` is `None`, `original_data` is `Null`.
  - Final `ResponseEnvelope<Vec<u8>>` has **empty** `original_data`.
- The DIMSE `QueryProvider` then receives a response with no body; any attempt to parse WADO multipart or DICOM instances will see zero bytes.

Effectively, C-GET and C-MOVE through this bridge cannot deliver any instances back to the DIMSE layer, even if the DICOMweb server returned correct data.

**Root cause:** the generic response JSON-conversion step occurs before the dicom_to_dicomweb middleware and the DIMSE integration, and it discards non-JSON bodies.

### 2.2 C-STORE DICOM status from STOW is ignored in the active QueryProvider

There are two `PipelineQueryProvider` implementations:

- `src/adapters/dimse/query_provider.rs` (active, used by `DimseAdapter`).
- `src/integrations/dimse/pipeline_query_provider.rs` (deprecated shim).

`dicom_to_dicomweb.transform_cstore_response` sets `response_details.metadata["dicom_status"]` based on HTTP status.

- The **deprecated** `integrations` provider’s `store()` reads `dicom_status` and turns DICOM failures into `DimseError`.
- The **active** `adapters::dimse::query_provider::PipelineQueryProvider::store()`:
  - Persists the file via storage backend.
  - Calls `self.run("C-STORE", body, meta)`.
  - Discards the `ResponseEnvelope` completely; it does **not** inspect `dicom_status` or HTTP status.

So, in the actual runtime path, C-STORE DIMSE callers will see success as long as the pipeline call doesn’t error at the transport level, regardless of STOW-RS failures.

## 3. Multipart / Binary Handling Gaps

### 3.1 WADO multipart parsing is stubbed and unsafe (deprecated path)

In `dicom_to_dicomweb.parse_multipart_wado_response`:

- Only extracts `boundary` and sets:
  - `response_format = "multipart/dicom"`.
  - `multipart_boundary = <boundary>`.
  - `dicom_status = SUCCESS`.
- Does NOT parse the body.

In the older `integrations/dimse/pipeline_query_provider::parse_multipart_body`:

- Converts the entire body to `String` using `String::from_utf8_lossy`.
- Splits on textual boundary markers.
- Trims and treats content as UTF-8 text, then re-encodes as bytes.

Problems:

- Arbitrary DICOM binary can contain boundary-like sequences.
- `from_utf8_lossy` can corrupt binary payloads.
- This approach is not robust enough for real WADO multipart responses.
- Combined with the C-GET/C-MOVE body-loss issue, multipart parsing is effectively non-functional today.

## 4. Protocol / Semantics Issues

### 4.1 Single HTTP→DICOM status mapping reused for all operations

`http_to_dicom_status` maps:

- `2xx` → `SUCCESS`.
- `400` → `FAILURE_IDENTIFIER_DOES_NOT_MATCH`.
- `404` → `SUCCESS` (treating “Not Found” as “no matches” for queries).
- `409` → `WARNING_SUBOPS_COMPLETE_WITH_FAILURES`.
- `413` → `FAILURE_OUT_OF_RESOURCES`.
- `5xx` and others → `FAILURE_UNABLE_TO_PROCESS`.

This makes sense for **C-FIND/QIDO** (404 = no matches), but is questionable for **C-STORE/C-GET/C-MOVE**:

- STOW 404 usually indicates config/endpoint errors, not “successful store with zero matches”.
- WADO 404 for C-GET/C-MOVE likely indicates an absent instance and should probably map to a failure status.

The current mapping is operation-agnostic and can under-report real failures as successes.

### 4.2 C-FIND → QIDO translation is minimal

`transform_cfind_request`:

- Walks the `identifier` JSON, turning tags with non-empty `Value` into query params (`tag=value`).
- Uses only the first value and does special handling for PN `{ "Alphabetic": "..." }`.
- Ignores `query_metadata` (match types: RETURN_KEY/WILDCARD/RANGE).
- Does not set `limit`/`offset` or `includefield`.

Result: works for simple queries, but doesn’t exploit QIDO’s richer semantics and may not scale well (no pagination and no includefield-based minimization).

### 4.3 Narrow C-GET / C-MOVE request mapping

`transform_cget_cmove_request`:

- Only inspects UIDs 0020000D (study), 0020000E (series), 00080018 (instance).
- Chooses one of:
  - `/studies/{study}/series/{series}/instances/{instance}`
  - `/studies/{study}/series/{series}`
  - `/studies/{study}`
  - `/studies`.
- Always sets `Accept: multipart/related; type="application/dicom"`.

Missing features:

- WADO metadata endpoints (`/metadata`).
- Rendered images (`Accept: image/jpeg` / `image/png`).
- Other WADO query options.

It’s a minimal implementation focused solely on raw DICOM instances.

### 4.4 Blocking file I/O in async C-STORE path

`build_stow_multipart` uses `std::fs::read` synchronously inside async middleware:

- Can block tokio worker threads on large objects or high concurrency.
- Ideally should use `tokio::fs::read` or `spawn_blocking`.

This is a performance/scheduling concern rather than logic bug.

## 5. Testing and Documentation Gaps

### 5.1 Right-side unit tests don’t cover the real logic

`tests/dicomweb/dicom_to_dicomweb_middleware.rs`:

- Tests only left‑hand transforms (C-FIND, C-STORE, C-GET, C-MOVE → HTTP).
- The `test_right_passthrough` still assumes the right side is a passthrough and only asserts that the status is unchanged.
- There are **no tests** exercising:
  - `transform_cfind_response` (arrays, single objects, null, HTTP error mapping).
  - `transform_cstore_response` mapping and failed instance reporting.
  - `transform_cget_cmove_response` and `parse_multipart_wado_response`.

The comment about “passthrough” is outdated relative to current implementation.

### 5.2 Integration tests are ignored and shallow on data validation

`tests/dicom_to_dicomweb_integration.rs`:

- All tests are `#[ignore]` and depend on DCMTK tools + mock DICOMweb server.
- They mainly validate:
  - Associations are accepted.
  - Commands don’t get rejected outright.
- They do **not** deeply assert that datasets are actually returned and correctly parsed on C-GET/C-MOVE.

Given the body-handling issues above, these tests likely wouldn’t catch that no datasets are delivered.

### 5.3 No dedicated docs for dicom_to_dicomweb

`docs/middleware.md` documents `dicomweb_bridge` (DICOMweb→DIMSE) but there is no section for `dicom_to_dicomweb` (DIMSE→DICOMweb).

- Users have to infer behavior from examples and code.
- Current limitations (C-GET/C-MOVE incomplete, C-STORE status not enforced, etc.) are undocumented.

## 6. Suggested Next Steps (Design-Level)

**Not implemented yet – this is a roadmap sketch.**

1. **Fix C-GET/C-MOVE data path**
   - Option A: Bypass JSON conversion for non-JSON DICOMweb content types in `PipelineExecutor::process_outgoing_middleware`.
   - Option B: In `HttpEndpoint`, for WADO responses, stash body bytes into `normalized_data` (e.g. `body_b64`) before `to_json` so `dicom_to_dicomweb` and the DIMSE layer can still access them.
   - Option C: Move WADO parsing into `dicom_to_dicomweb` while it still sees `Vec<u8>` (requires changing middleware plumbing to operate on bytes for that middleware).

2. **Wire C-STORE status through the active QueryProvider**
   - After `run("C-STORE", ...)`, inspect `response.response_details.metadata["dicom_status"]` and map C/A codes to `DimseError` for failure cases.
   - Keep deprecated implementation as reference for expected behavior.

3. **Introduce operation-specific HTTP→DICOM status mapping**
   - Split `http_to_dicom_status` into variants per DIMSE op, or pass op into the mapper.
   - Treat 404 as success only for C-FIND, and as failure (or at least warning) for C-STORE / C-GET / C-MOVE.

4. **Refine QIDO and WADO request mapping**
   - Use `query_metadata` (match types) to better encode wildcards/ranges.
   - Map `max_results` to `limit` and optionally support `offset`.
   - Consider using named QIDO params (`PatientName`, `PatientID`, etc.) when appropriate.

5. **Harden multipart parsing**
   - Replace the string-based multipart parser with a binary-safe parser (e.g. using `mime_multipart`-style parsing or `httparse` + manual boundary scanning on bytes).
   - Ensure we never round-trip DICOM payloads through UTF‑8 conversions.

6. **Improve tests and documentation**
   - Add unit tests for right-hand transformations (`transform_cfind_response`, `transform_cstore_response`, `transform_cget_cmove_response`).
   - Enable at least one C-GET/C-MOVE integration test in CI that verifies datasets are actually written to storage or returned.
   - Add a dedicated `dicom_to_dicomweb` section to `docs/middleware.md` describing:
     - Supported operations.
     - Status mapping behavior.
     - Known limitations and TODOs.
