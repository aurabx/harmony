# Pipeline Implementation Roadmap

**Date**: 2025-01-12 (Updated: 2025-11-12)  
**Goal**: Enable 12 pipeline types across Data Integration, Healthcare, and API Bridge domains  
**Timeline**: 12 weeks (3 phases)  
**Estimated Effort**: ~6,700 LOC + tests + docs

**Status**: Phase 1 partially complete - Multi-content-type support implemented (2025-11-12)

---

## Executive Summary

This roadmap prioritizes **multi-purpose components** to maximize early pipeline completions. By building foundational components first (CSV, XML, Database, JSON Validation), we can deliver **5 complete pipelines within 3 weeks**.

## Pipeline Completion Schedule

| Week | Cumulative Pipelines | New Completions | Status |
|------|---------------------|-----------------|--------|
| 3    | 3 pipelines         | CSV→JSON/XML, DB→REST, Webhook→DB | 🟡 1/3 parsers ready |
| 6    | 7 pipelines         | HL7v2→FHIR, DICOM anon, EHR→warehouse, X12 claims | ⏳ Pending |
| 10   | 12 pipelines        | REST→GraphQL, SOAP→REST, gRPC→HTTP, WebSocket→HTTP, Multi-source agg | ⏳ Pending |

**Note**: Multi-content-type foundation (2025-11-12) enables CSV/XML/form/multipart parsing for multiple pipelines.

---

## Component Reusability Matrix

| Component | Serves Pipelines | Build Priority | Status |
|-----------|-----------------|----------------|--------|
| **JSON Validation** | 8+ pipelines | 🔴 CRITICAL | ⏳ Pending |
| **Database Backend** | 4 pipelines (#2, #3, #4, #7) | 🔴 CRITICAL | ⏳ Pending |
| **XML Handler** | 3 pipelines (#1, #5, #10) | 🔴 CRITICAL | ✅ **COMPLETE** |
| **CSV Parser** | 2 pipelines (#1, #4) | 🔴 CRITICAL | ✅ **COMPLETE** |
| **Form Data Parser** | Multiple pipelines | 🔴 CRITICAL | ✅ **COMPLETE** |
| **Multipart Handler** | File upload pipelines | 🔴 CRITICAL | ✅ **COMPLETE** |
| **Binary Content** | File/image pipelines | 🔴 CRITICAL | ✅ **COMPLETE** |
| **Webhook Endpoint** | 2 pipelines (#3, #7) | 🟡 HIGH | ⏳ Pending |
| HL7v2 Parser | 1 pipeline (#5) | 🟡 HIGH | ⏳ Pending |
| X12 Parser | 1 pipeline (#8) | 🟡 HIGH | ⏳ Pending |
| DICOM Anonymization | 1 pipeline (#6) | 🟢 MEDIUM | ⏳ Pending |
| GraphQL | 1 pipeline (#9) | 🟢 MEDIUM | ⏳ Pending |
| SOAP | 1 pipeline (#10) | 🟢 MEDIUM | ⏳ Pending |
| gRPC Adapter | 1 pipeline (#11) | 🟢 MEDIUM | ⏳ Pending |
| WebSocket Adapter | 1 pipeline (#12) | 🟢 MEDIUM | ⏳ Pending |

---

## Phase 1: Foundation (Weeks 1-3) ✅ PARTIALLY COMPLETE

**Goal**: Enable 3+ pipelines with multi-purpose components  
**Status**: Multi-content-type support implemented (2025-11-12)

### Completed Components ✅

#### ✅ 1. Multi-Content-Type Parsing (`src/adapters/http/content_type.rs`)
- **Status**: ✅ **COMPLETE** (2025-11-12)
- **LOC**: 540 lines (implemented)
- **Dependencies**: 
  - `csv` 1.3
  - `quick-xml` 0.31 (with serialize feature)
  - `multer` 3.0 (multipart)
  - `encoding_rs` 0.8
  - `sha2` 0.10
  - `lazy_static` 1.4
  - `bytes` 1.5
- **Enables**: Pipelines #1 (CSV→JSON/XML), #4 (multi-source), file uploads, form processing
- **Implemented Parsers**:
  1. **JSON** - `application/json`, `application/fhir+json`, `application/dicom+json`
  2. **XML** - `application/xml`, `text/xml`, `application/soap+xml`
  3. **CSV** - `text/csv` with formula injection prevention
  4. **Form URL-encoded** - `application/x-www-form-urlencoded` with array support
  5. **Multipart** - `multipart/form-data` with file metadata
  6. **Binary** - `image/*`, `video/*`, `audio/*`, `application/pdf`, etc.

- **Interface** (Implemented):
  ```rust
  // Content-Type header parsing
  pub fn parse_content_type(header: &str) -> Result<ContentType, Error>;
  
  // Format parsers
  pub fn parse_xml(body: &[u8]) -> Result<Value, Error>;
  pub fn parse_csv(body: &[u8]) -> Result<Value, Error>;
  pub fn parse_form_urlencoded(body: &[u8]) -> Result<Value, Error>;
  pub async fn parse_multipart(body: &[u8], boundary: Option<String>) -> Result<Value, Error>;
  pub fn calculate_checksum(data: &[u8]) -> String;
  pub fn create_binary_metadata(media_type: &str, data: &[u8]) -> Value;
  
  // With configurable limits
  pub fn parse_csv_with_limit(body: &[u8], max_rows: usize) -> Result<Value, Error>;
  pub fn parse_xml_with_limit(body: &[u8], max_depth: usize) -> Result<Value, Error>;
  pub async fn parse_multipart_with_limit(body: &[u8], boundary: Option<String>, max_files: usize) -> Result<Value, Error>;
  pub fn parse_form_urlencoded_with_limit(body: &[u8], max_fields: usize) -> Result<Value, Error>;
  ```

- **Security Features**:
  - XXE prevention (XML - no external entities)
  - CSV formula injection mitigation (prefix dangerous characters)
  - XML bomb prevention (depth limits)
  - Multipart file limits
  - Form field count limits
  - Configurable size limits

- **Configuration** (`src/config/proxy_config.rs`):
  ```rust
  pub struct ContentLimits {
      pub max_body_size: usize,         // Default: 10MB
      pub max_csv_rows: usize,           // Default: 10,000
      pub max_xml_depth: usize,          // Default: 100
      pub max_multipart_files: usize,    // Default: 10
      pub max_form_fields: usize,        // Default: 1,000
  }
  ```

- **Envelope Integration** (`src/models/envelope/envelope.rs`):
  ```rust
  pub struct ContentMetadata {
      pub content_type: String,
      pub charset: Option<String>,
      pub format: String,                // json, xml, csv, form, multipart, binary
      pub parse_status: ParseStatus,     // Success, Failed, NotAttempted, Unsupported
      pub original_size: usize,
      pub checksum: Option<String>,      // SHA256 for binary content
  }
  
  pub struct RequestDetails {
      // ... existing fields ...
      pub content_metadata: Option<ContentMetadata>,
  }
  ```

- **Testing**:
  - ✅ 7 unit tests (parser functions)
  - ✅ 14 integration tests (end-to-end pipelines)
  - ✅ Security tests (XXE, formula injection)
  - ✅ 266 total tests passing

- **Documentation**:
  - ✅ [docs/content-types.md](../../docs/content-types.md) (609 lines)
  - ✅ Examples: `examples/content-types/`
  - ✅ README.md updated
  - ✅ warp.md updated

### Remaining Phase 1 Components

#### 2. JSON Schema Validation (`src/validation/json_schema.rs`) ⏳ PENDING
- **LOC**: ~200 (wrapper around `jsonschema` crate)
- **Dependencies**: `jsonschema` (0.17)
- **Enables**: All JSON-based pipelines (8+)
- **Config Integration**:
  ```toml
  [middleware.json_validator]
  type = "validation"
  [middleware.json_validator.options]
  schema_path = "schemas/fhir-patient.json"
  fail_on_error = true
  ```

#### 3. Database Backend Service (`src/services/database.rs`) ⏳ PENDING
- **LOC**: ~600
- **Dependencies**: `sqlx` (0.7) with postgres, mysql, sqlite features
- **Enables**: Pipeline #2 (DB→REST), #3 (Webhook→DB), #4 (agg), #7 (warehouse)
- **Supports**: PostgreSQL, MySQL, SQLite, MongoDB
- **Interface**:
  ```rust
  #[async_trait]
  pub trait DatabaseService {
      async fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<JsonValue>>;
      async fn execute(&self, sql: &str, params: Vec<Value>) -> Result<u64>;
      async fn insert(&self, table: &str, data: &JsonValue) -> Result<u64>;
  }
  ```

#### 4. Webhook Endpoint (`src/endpoints/webhook.rs`) ⏳ PENDING
- **LOC**: ~300
- **Dependencies**: None (uses existing HTTP adapter)
- **Enables**: Pipeline #3 (Webhook→DB), #7 (EHR webhook)
- **Features**:
  - HMAC-SHA256 signature verification
  - Rate limiting per sender
  - Replay attack prevention
  - Automatic JSON/form-data parsing

### Deliverables (Week 3)

**Phase 1 Status**: 🟡 Partially Complete

✅ **Completed**:
- Multi-content-type parsing infrastructure
- CSV, XML, form data, multipart, binary support
- Security features (XXE prevention, CSV sanitization)
- Content metadata tracking
- Configurable limits
- Comprehensive documentation

⏳ **Remaining**:
- JSON Schema Validation middleware
- Database Backend Service
- Webhook Endpoint with HMAC verification

**Pipeline Status**:
1. ✅ CSV to JSON/XML - **ENABLED** (can parse CSV/XML, needs validation middleware)
2. ⏳ Database to REST API sync - Pending database backend
3. ⏳ Webhook to Database - Pending webhook endpoint + database backend

---

## Phase 2: Healthcare (Weeks 4-6)

**Goal**: Add healthcare-specific format handlers

### Components

#### 6. HL7v2 Parser (`src/formats/hl7v2.rs`)
- **LOC**: ~800 (custom implementation)
- **Enables**: Pipeline #5 (HL7v2→FHIR)
- **Message Types**: ADT, ORU (expand iteratively)
- **Interface**:
  ```rust
  pub struct Hl7Message {
      pub segments: Vec<Hl7Segment>,
      pub message_type: String,
      pub version: String,
  }
  
  pub trait Hl7Parser {
      fn parse(&self, hl7_text: &str) -> Result<Hl7Message>;
      fn to_json(&self, msg: &Hl7Message) -> Result<JsonValue>;
  }
  ```

#### 7. X12 Claims Parser (`src/formats/x12.rs`)
- **LOC**: ~1000 (custom implementation)
- **Enables**: Pipeline #8 (X12 claims)
- **Transaction Sets**: 837 (claims), 835 (remittance), 834 (enrollment)
- **Interface**:
  ```rust
  pub struct X12Document {
      pub transaction_set: String,
      pub segments: Vec<X12Segment>,
      pub loops: Vec<X12Loop>,
  }
  ```

#### 8. DICOM Anonymization Middleware (`src/middleware/dicom_anonymization.rs`)
- **LOC**: ~500
- **Dependencies**: `dicom-rs` (already in use)
- **Enables**: Pipeline #6 (DICOM anonymization)
- **Profiles**: Basic, Clean, Retain Dates (DICOM PS3.15 compliant)
- **Config**:
  ```toml
  [middleware.dicom_phi_removal]
  type = "dicom_anonymization"
  [middleware.dicom_phi_removal.options]
  profile = "basic"
  hash_uids = true
  tag_whitelist = ["(0010,0020)"]  # Patient ID
  ```

#### 9. FHIR Transformation Module (`src/formats/fhir.rs`)
- **LOC**: ~300 (JOLT spec helpers)
- **Enables**: Pipeline #5 (HL7v2→FHIR)
- **Approach**: Leverage existing JOLT transform middleware
- **Features**: FHIR R4 validation, output schema validation

### Deliverables (Week 6)

✅ **7 Complete Pipelines** (4 new):
5. HL7v2 to FHIR transformation
6. DICOM anonymization pipeline
7. EHR webhook to analytics warehouse
8. Claims data (X12) processing

---

## Phase 3: API Bridges (Weeks 7-10)

**Goal**: Add protocol adapters and advanced orchestration

### Components

#### 10. GraphQL Endpoint + Backend (`src/endpoints/graphql.rs`, `src/backends/graphql.rs`)
- **LOC**: ~400 + ~200
- **Dependencies**: `async-graphql` (7.0)
- **Enables**: Pipeline #9 (REST→GraphQL)
- **Features**: Schema introspection, query depth limiting

#### 11. SOAP Endpoint (`src/endpoints/soap.rs`)
- **LOC**: ~500
- **Dependencies**: `quick-xml` + custom WSDL parser
- **Enables**: Pipeline #10 (SOAP→REST)
- **Features**: SOAP envelope parsing, WSDL validation

#### 12. gRPC Adapter (`src/adapters/grpc.rs`)
- **LOC**: ~600
- **Dependencies**: `tonic` (0.11), `prost` (0.12)
- **Enables**: Pipeline #11 (gRPC→HTTP/JSON)
- **Features**: gRPC reflection, protobuf→JSON conversion

#### 13. WebSocket Adapter (`src/adapters/websocket.rs`)
- **LOC**: ~700
- **Dependencies**: `tokio-tungstenite` (0.21)
- **Enables**: Pipeline #12 (WebSocket→HTTP)
- **Features**: Connection state management, long-polling simulation

#### 14. Aggregation Middleware (`src/middleware/aggregation.rs`)
- **LOC**: ~400
- **Enables**: Pipeline #4 (completes multi-source aggregation)
- **Strategies**: Sequential, Parallel, Fallback
- **Features**: Multi-backend fan-out, response merging (union/intersection/custom JOLT)

### Deliverables (Week 10)

✅ **12 Complete Pipelines** (5 new):
4. Multi-source data aggregation (completed)
9. REST to GraphQL
10. SOAP to REST modernization
11. gRPC to HTTP/JSON
12. WebSocket to HTTP long-polling

---

## Implementation Patterns

### Pattern 1: Format Handler (New)

**Location**: `src/formats/registry.rs`

```rust
pub trait FormatHandler: Send + Sync {
    fn name(&self) -> &str;
    fn parse(&self, data: &[u8]) -> Result<JsonValue>;
    fn serialize(&self, data: &JsonValue) -> Result<Vec<u8>>;
    fn validate(&self, data: &JsonValue, schema: Option<&Path>) -> Result<()>;
}

pub struct FormatRegistry {
    handlers: HashMap<String, Box<dyn FormatHandler>>,
}
```

**Implementations**: CSV, XML, HL7v2, X12

---

### Pattern 2: Storage Backend (New)

**Location**: `src/backends/storage.rs`

```rust
#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn read(&self, key: &str) -> Result<Vec<u8>>;
    async fn write(&self, key: &str, data: &[u8]) -> Result<()>;
    async fn delete(&self, key: &str) -> Result<()>;
    async fn list(&self, prefix: &str) -> Result<Vec<String>>;
}

pub enum StorageType {
    Database(Box<dyn DatabaseService>),
    Filesystem(PathBuf),
    S3 { bucket: String, region: String },
    AzureBlob { container: String },
}
```

---

### Pattern 3: Protocol Adapter (Existing - Extend)

**Location**: `src/adapters/mod.rs`

**Existing Interface**:
```rust
#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    async fn start(&self, config: Arc<Config>, shutdown: CancellationToken) -> Result<JoinHandle<()>>;
    fn protocol(&self) -> Protocol;
}
```

**New Implementations**: `GrpcAdapter`, `WebSocketAdapter`

---

### Pattern 4: Envelope Extension (Partially Implemented)

**Current**: `src/models/envelope.rs`

**✅ Implemented** (2025-11-12):
```rust
pub struct RequestDetails {
    // ... existing fields ...
    pub content_metadata: Option<ContentMetadata>,  // NEW: Tracks parsing status
}

pub struct ContentMetadata {
    pub content_type: String,
    pub charset: Option<String>,
    pub format: String,                // json, xml, csv, form, multipart, binary
    pub parse_status: ParseStatus,
    pub original_size: usize,
    pub checksum: Option<String>,
}

pub enum ParseStatus {
    Success,
    Failed,
    NotAttempted,
    Unsupported,
}
```

**⏳ Proposed Extensions** (Future):
```rust
pub struct RequestEnvelope<T> {
    pub request_details: RequestDetails,
    pub original_data: T,
    pub normalized_data: Option<Value>,
    pub normalized_snapshot: Option<Value>,
    
    // FUTURE: Type-specific data fields
    pub dicom_data: Option<DicomObject>,
    pub hl7_message: Option<Hl7Message>,
    pub x12_document: Option<X12Document>,
    pub graphql_context: Option<GraphQLContext>,
}
```

---

## Existing Components (No New Work)

✅ **HTTP Adapter** - Already supports REST, webhooks  
✅ **Multi-Content-Type Parsing** - **NEW** (2025-11-12) - JSON, XML, CSV, form, multipart, binary  
✅ **DIMSE Adapter** - Already supports DICOM SCP/SCU  
✅ **JOLT Transform Middleware** - Already handles JSON transforms  
✅ **JWT/Basic Auth Middleware** - Already handles authentication  
✅ **PipelineExecutor** - Already orchestrates request/response flow  
✅ **Config Hot-Reload** - Already supports zero-downtime updates  
✅ **Content Security** - **NEW** (2025-11-12) - XXE prevention, CSV injection mitigation, size limits  

---

## Risk Assessment

### High Risk Components

| Component | Risk | Mitigation |
|-----------|------|------------|
| **HL7v2 Parser** | Complex spec, healthcare domain expertise | Start with ADT/ORU subset, expand iteratively |
| **X12 Parser** | Commercial crates limited | Custom implementation for 837 only initially |
| **gRPC→JSON** | Type mapping complexity | Use gRPC reflection API for runtime discovery |
| **Aggregation** | Deadlock risk with parallel requests | Tokio timeouts + cancellation tokens, circuit breaker |

### Medium Risk Components

| Component | Risk | Mitigation |
|-----------|------|------------|
| **DICOM Anonymization** | DICOM PS3.15 compliance | Use dicom-rs examples as baseline, extensive testing |
| **WebSocket Management** | Memory leaks with long-lived connections | Connection limits + idle timeout, memory profiling |
| **SOAP WSDL** | Legacy standard, limited tooling | Custom parser using quick-xml |

### Low Risk Components

✅ CSV/XML/JSON handling - **COMPLETE** - Implemented with security features  
✅ Form data parsing - **COMPLETE** - URL-encoded and multipart  
✅ Binary content handling - **COMPLETE** - Checksum and metadata  
✅ Database abstraction - sqlx provides solid foundation  
✅ GraphQL - async-graphql handles complexity  

---

## Development Resources

### Estimated Effort

| Category | LOC | Tests | Docs | Status |
|----------|-----|-------|------|--------|
| Multi-content-type | 540 | 21 tests | 609 lines | ✅ Complete |
| Config Extensions | 100 | - | - | ✅ Complete |
| Remaining New Code | ~6,160 | ~2,979 | ~4,400 words | ⏳ Pending |
| **Total** | **~6,800** | **~3,000** | **~5,000 words** | 🟡 ~10% Complete |

### Team Composition (Recommended)

- **2 full-time Rust developers** (general components)
- **1 healthcare domain expert** (HL7/FHIR/X12) - Weeks 4-8
- **1 infrastructure engineer** (databases, protocols) - Weeks 2-10
- **1 technical writer** (part-time) - Weeks 11-12

**Alternative (smaller team)**: 2 developers + healthcare consultant (as needed)

---

## Testing Strategy

### Unit Tests (~120 tests)

- Format handlers: CSV, XML, HL7v2, X12 parsing/serialization
- Database service: Query, insert, transaction handling
- Middleware: Validation, aggregation, anonymization
- Protocol adapters: gRPC, WebSocket frame handling

### Integration Tests (~35 tests)

- End-to-end pipeline tests (all 12 types)
- Multi-backend aggregation
- Error handling (invalid data, backend failures)
- Authentication flows

### Performance Tests (~10 tests)

- Throughput: Requests/second per pipeline type
- Latency: P50, P95, P99 response times
- Memory: Connection pooling, long-lived WebSocket connections
- Database: Query performance under load

---

## Example: Pipeline #1 Configuration

**CSV to JSON/XML with Validation**

```toml
[pipelines.csv_transform]
description = "CSV to JSON/XML with validation"
networks = ["default"]
endpoints = ["csv_upload"]
backends = ["json_api", "xml_api"]
middleware = ["csv_parser", "json_validator", "format_selector"]

[endpoints.csv_upload]
service = "http"
[endpoints.csv_upload.options]
path = "/upload/csv"
methods = ["POST"]
max_body_size = 10485760  # 10MB

[middleware.csv_parser]
type = "transform"
[middleware.csv_parser.options]
spec_path = "transforms/csv-to-json.json"

[middleware.json_validator]
type = "validation"
[middleware.json_validator.options]
schema_path = "schemas/output.json"

[middleware.format_selector]
type = "transform"
apply = "right"
[middleware.format_selector.options]
spec_path = "transforms/json-to-xml.json"  # Applied if Accept: application/xml

[backends.json_api]
service = "http"
[backends.json_api.options]
base_url = "https://api.example.com/json"

[backends.xml_api]
service = "http"
[backends.xml_api.options]
base_url = "https://api.example.com/xml"
```

---

## Next Steps

1. **Review & Approve**: Stakeholder sign-off on component priorities
2. **Sprint Planning**: Break down Phase 1 into 2-week sprints
3. **Crate Evaluation**: Test HL7v2/X12 parser crates (vs. custom implementation)
4. **Prototype**: Build CSV handler + JSON validation as proof of concept
5. **Team Assembly**: Hire/assign healthcare domain expert for Phase 2

---

## References

- **Existing Documentation**:
  - [docs/adapters.md](../../docs/adapters.md) - Protocol adapter architecture
  - [docs/endpoints.md](../../docs/endpoints.md) - Endpoint types
  - [docs/backends.md](../../docs/backends.md) - Backend services
  - [docs/middleware.md](../../docs/middleware.md) - Middleware chain
  - [docs/envelope.md](../../docs/envelope.md) - Data exchange format

- **Internal Documentation**:
  - [docs/content-types.md](../../docs/content-types.md) - **NEW** Multi-content-type support guide
  - [examples/content-types/](../../examples/content-types/) - **NEW** Usage examples

- **External Resources**:
  - HL7 v2.x Specification: https://www.hl7.org/implement/standards/product_brief.cfm?product_id=185
  - X12 EDI Standards: https://x12.org/products/transaction-sets
  - DICOM PS3.15 Anonymization: https://dicom.nema.org/medical/dicom/current/output/html/part15.html
  - FHIR R4: https://www.hl7.org/fhir/

---

**Document Version**: 1.1  
**Last Updated**: 2025-11-12  
**Status**: In Progress - Phase 1 partially complete  

---

## Change Log

### 2025-11-12 - Multi-Content-Type Implementation Complete

**Completed**:
- ✅ Multi-content-type parsing (JSON, XML, CSV, form, multipart, binary)
- ✅ Content security features (XXE prevention, CSV sanitization, limits)
- ✅ Content metadata tracking in envelope
- ✅ Configurable limits (ContentLimits struct)
- ✅ Comprehensive test coverage (266 tests passing)
- ✅ Full documentation (docs/content-types.md)
- ✅ Example configurations

**Impact**:
- Enables Pipeline #1 (CSV→JSON/XML) - Parser ready, needs validation middleware
- Enables file upload pipelines
- Enables form processing pipelines
- Foundation for Pipeline #4 (multi-source aggregation)
- Foundation for Pipeline #10 (SOAP→REST)

**Next Steps**:
- Implement JSON Schema Validation middleware
- Implement Database Backend Service
- Implement Webhook Endpoint
- Complete Pipeline #1 end-to-end testing

**Files Modified/Created**:
- `src/adapters/http/content_type.rs` (540 lines)
- `src/models/envelope/envelope.rs` (ContentMetadata added)
- `src/config/proxy_config.rs` (ContentLimits added)
- `src/models/services/types/http.rs` (integrated content-type routing)
- `tests/http/content_type_integration.rs` (14 integration tests)
- `docs/content-types.md` (609 lines)
- `examples/content-types/` (config + README)
- `Cargo.toml` (7 new dependencies)
