# DIMSE Crate & Adapter Refactoring Review

**Review Date**: 2025-01-21  
**Codebase**: `crates/dimse` and `src/adapters/dimse`  
**Lines of Code**: 
- `crates/dimse/src/scp.rs`: 1,525 lines
- `crates/dimse/src/scu.rs`: 798 lines
- `src/adapters/dimse/mod.rs`: 528 lines
- `crates/dimse/src/router.rs`: 470 lines

## Executive Summary

The DIMSE crate provides a solid foundation for DICOM networking with both SCP (Service Class Provider) and SCU (Service Class User) implementations. However, several modules have grown large and would benefit from refactoring to improve maintainability, testability, and code organization.

**Overall Assessment**: ⭐⭐⭐ (3/5)
- **Strengths**: Functional implementation, good error types, comprehensive protocol support
- **Areas for Improvement**: Module organization, code duplication, incomplete features, unused abstractions

---

## 1. SCP Module is Too Large (scp.rs: 1,525 lines)

### Issue

The `scp.rs` file contains multiple responsibilities:
- Association lifecycle management
- PDU parsing and handling
- Command routing (`dispatch_command`)
- Individual command handlers (C-ECHO, C-FIND, C-MOVE, C-GET, C-STORE)
- Response building and encoding
- Query parameter extraction

This violates the Single Responsibility Principle and makes the code difficult to navigate, test, and maintain.

### Current Structure

```markdown:./dev/analysis/dimse-refactoring.md
<code_block_to_apply_changes_from>
scp.rs (1,525 lines)
├── DimseScp struct
├── QueryProvider trait
├── handle_association() - 100+ lines
├── handle_pdata() - 130+ lines
├── dispatch_command() - 60 lines
├── handle_c_echo() - 82 lines
├── handle_c_find() - 140 lines
├── send_cfind_response() - 108 lines
├── handle_c_move() - 58 lines
├── send_cmove_response() - 100 lines
├── handle_c_get() - 36 lines
├── send_cget_response() - 100 lines
├── handle_c_store() - 67 lines
└── send_cstore_response() - 68 lines
```

### Refactoring Proposal

Split into focused modules:

```
scp/
├── mod.rs              # Main SCP struct, lifecycle, public API
├── association.rs      # Association establishment and management
├── pdu_handler.rs      # PDU parsing, accumulation, routing
├── commands/
│   ├── mod.rs          # Command dispatcher and registry
│   ├── echo.rs         # C-ECHO handler
│   ├── find.rs         # C-FIND handler
│   ├── move.rs         # C-MOVE handler
│   ├── get.rs          # C-GET handler
│   └── store.rs        # C-STORE handler
└── response_builder.rs # Response encoding helpers
```

**Benefits**:
- ✅ Easier navigation (each file < 200 lines)
- ✅ Better testability (isolate command handlers)
- ✅ Clear separation of concerns
- ✅ Parallel development on different commands

**Priority**: 🔴 High  
**Effort**: Medium (3-4 days)

---

## 2. SCU Module Has Significant Code Duplication

### Issue

The `scu.rs` file contains three nearly identical implementations for building DCMTK command arguments:
- `find_impl()` - builds `findscu` args
- `move_impl()` - builds `movescu` args  
- `get_impl()` - builds `getscu` args

Each method:
1. Builds base arguments (local AET, remote AET, host, port)
2. Adds query level parameter
3. Adds query parameters (with tag format conversion)
4. Sets output directory
5. Spawns async task to run command
6. Reads output files and streams results
7. Cleans up temporary directories

### Current Duplication

```rust
// find_impl() - lines 97-219
let mut args: Vec<String> = vec![
    "-aet".into(), self.config.local_aet.clone(),
    "-aec".into(), node.ae_title.clone(),
    // ... query level, parameters, output dir
];

// move_impl() - lines 251-428  
let mut args: Vec<String> = vec![
    "-aet".into(), self.config.local_aet.clone(),
    "-aec".into(), node.ae_title.clone(),
    // ... similar pattern
];

// get_impl() - lines 460-598
let mut args: Vec<String> = vec![
    "-aet".into(), self.config.local_aet.clone(),
    "-aec".into(), node.ae_title.clone(),
    // ... similar pattern
];
```

### Refactoring Proposal

Create a shared DCMTK command builder:

```rust
// scu/dcmtk_builder.rs
pub struct DcmtkCommandBuilder {
    local_aet: String,
    storage_dir: PathBuf,
}

impl DcmtkCommandBuilder {
    /// Build base arguments common to all commands
    pub fn build_base_args(&self, node: &RemoteNode, command: &str) -> Vec<String> {
        let mut args = match command {
            "find" => vec!["-P".into()],  // Patient Root for find
            "move" => vec!["-S".into(), "-d".into()],  // Study Root, debug
            "get" => vec![],
            _ => vec![],
        };
        
        args.extend(vec![
            "-aet".into(), self.local_aet.clone(),
            "-aec".into(), node.ae_title.clone(),
        ]);
        args
    }
    
    /// Add query level parameter
    pub fn add_query_level(&self, args: &mut Vec<String>, level: QueryLevel, command: &str) {
        let level_str = match level {
            QueryLevel::Patient => "PATIENT",
            QueryLevel::Study => "STUDY",
            QueryLevel::Series => "SERIES",
            QueryLevel::Image => "IMAGE",
        };
        
        match command {
            "find" => {
                args.push("-k".into());
                args.push(format!("QueryRetrieveLevel={}", level_str));
            }
            "move" | "get" => {
                args.push("-k".into());
                args.push(format!("0008,0052={}", level_str));
            }
            _ => {}
        }
    }
    
    /// Convert tag format (8-char to (gggg,eeee))
    pub fn normalize_tag(&self, tag: &str) -> String {
        if tag.len() == 8 {
            format!("{},{}", &tag[0..4], &tag[4..8])
        } else {
            tag.to_string()
        }
    }
    
    /// Add query parameters
    pub fn add_query_params(&self, args: &mut Vec<String>, params: &HashMap<String, String>) {
        for (k, v) in params {
            let tag = self.normalize_tag(k);
            args.push("-k".into());
            if v.is_empty() {
                args.push(format!("{}=", tag));
            } else {
                args.push(format!("{}={}", tag, v));
            }
        }
    }
    
    /// Build findscu command
    pub fn build_find_args(
        &self,
        node: &RemoteNode,
        query: &FindQuery,
    ) -> Vec<String> {
        let mut args = self.build_base_args(node, "find");
        self.add_query_level(&mut args, query.query_level, "find");
        self.add_query_params(&mut args, &query.parameters);
        // Add output directory logic
        args
    }
    
    /// Build movescu command
    pub fn build_move_args(
        &self,
        node: &RemoteNode,
        query: &MoveQuery,
        output_dir: Option<PathBuf>,
    ) -> Vec<String> {
        let mut args = self.build_base_args(node, "move");
        args.push("-aem".into());
        args.push(query.destination_aet.clone());
        self.add_query_level(&mut args, query.query_level, "move");
        self.add_query_params(&mut args, &query.parameters);
        // Add output directory and +P listener logic
        args
    }
    
    /// Build getscu command
    pub fn build_get_args(
        &self,
        node: &RemoteNode,
        query: &GetQuery,
        output_dir: Option<PathBuf>,
    ) -> Vec<String> {
        let mut args = self.build_base_args(node, "get");
        self.add_query_level(&mut args, query.query_level, "get");
        self.add_query_params(&mut args, &query.parameters);
        // Add output directory logic
        args
    }
}
```

**Benefits**:
- ✅ DRY principle - single source for command building
- ✅ Easier maintenance - changes in one place
- ✅ Consistent behavior across commands
- ✅ Easier to add new commands

**Priority**: 🔴 High  
**Effort**: Low-Medium (1-2 days)

---

## 3. Router Abstraction is Unused

### Issue

The `router.rs` module defines a `Router` trait and `InMemoryRouter` implementation, but:
- The SCP doesn't use the router abstraction
- There's a `handle_dimse_request()` method marked `#[allow(dead_code)]` that suggests incomplete refactoring
- The router was designed to decouple DIMSE operations from HTTP layer, but this integration isn't implemented

### Current State

```rust
// scp.rs:1240-1243
#[allow(dead_code)]
async fn handle_dimse_request(
    &self,
    request: DimseRequest,
    router: &Arc<dyn Router>,
) -> Result<()> {
    // This method exists but is never called
}
```

### Refactoring Options

**Option A: Remove Router (if not needed)**
- Remove unused router code
- Simplify codebase
- Risk: If router was intended for future HTTP integration, this removes that path

**Option B: Integrate Router into SCP**
- Refactor SCP to use router for command handling
- Better separation between network layer and business logic
- Enables future HTTP integration

**Option C: Document as Future Work**
- Keep router but document it's not currently used
- Mark for future HTTP layer integration

### Recommendation

**Option B** - Refactor SCP to use router:

```rust
// New structure:
scp/
├── mod.rs              # DimseScp with router integration
├── network_layer.rs   # Handles associations, PDU parsing
└── command_processor.rs  # Uses router to dispatch commands

// Flow:
// Network Layer → Parse PDU → Create DimseRequest → Router → Command Processor
```

**Benefits**:
- ✅ Clean separation of network and business logic
- ✅ Enables HTTP layer integration
- ✅ Better testability (mock router)
- ✅ Consistent with original design intent

**Priority**: 🟡 Medium  
**Effort**: Medium-High (3-5 days)

---

## 4. DatasetStream Conversion Incomplete

### Issue

The `DatasetStream` enum has placeholder implementations for conversions:

```rust
// types.rs:232-240
pub async fn to_object(&self) -> Result<InMemDicomObject> {
    match self {
        Self::Object { object, .. } => Ok(object.clone()),
        _ => {
            // TODO: Implement proper DICOM object parsing
            Ok(dicom_object::InMemDicomObject::new_empty())  // Placeholder!
        }
    }
}

// types.rs:216-228
pub async fn to_bytes(&self) -> Result<Bytes> {
    match self {
        Self::Memory { data, .. } => Ok(data.clone()),
        Self::File { path, .. } => {
            let bytes = tokio::fs::read(path).await?;
            Ok(Bytes::from(bytes))
        }
        Self::Object { .. } => {
            // TODO: Implement proper DICOM object serialization
            Ok(Bytes::new())  // Placeholder!
        }
    }
}
```

### Impact

- C-FIND responses may fail if dataset needs conversion
- C-STORE operations may not serialize correctly
- Memory/File/Object conversions are incomplete

### Refactoring Proposal

```rust
impl DatasetStream {
    pub async fn to_object(&self) -> Result<InMemDicomObject> {
        match self {
            Self::Object { object, .. } => Ok(object.clone()),
            Self::Memory { data, .. } => {
                // Parse from bytes
                let ts = TransferSyntaxRegistry
                    .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
                    .ok_or_else(|| DimseError::parse("Transfer syntax not found"))?;
                
                InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
                    std::io::Cursor::new(&*data),
                    ts,
                    SpecificCharacterSet::default(),
                )
                .map_err(|e| DimseError::parse(format!("Failed to parse dataset: {}", e)))
            }
            Self::File { path, .. } => {
                let bytes = tokio::fs::read(path).await?;
                // Similar parsing logic
                // ...
            }
        }
    }
    
    pub async fn to_bytes(&self) -> Result<Bytes> {
        match self {
            Self::Memory { data, .. } => Ok(data.clone()),
            Self::File { path, .. } => {
                let bytes = tokio::fs::read(path).await?;
                Ok(Bytes::from(bytes))
            }
            Self::Object { object, .. } => {
                // Serialize using dicom-object
                let ts = TransferSyntaxRegistry
                    .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
                    .ok_or_else(|| DimseError::operation_failed("Transfer syntax not found"))?;
                
                let mut bytes = Vec::new();
                object.write_dataset_with_ts(&mut bytes, ts)
                    .map_err(|e| DimseError::operation_failed(format!("Failed to serialize: {}", e)))?;
                Ok(Bytes::from(bytes))
            }
        }
    }
}
```

**Priority**: 🔴 High  
**Effort**: Low (1 day)

---

## 5. DimseAdapter Module is Too Large (mod.rs: 528 lines)

### Issue

`src/adapters/dimse/mod.rs` contains:
- Adapter trait implementation
- SCP registry management
- SCP startup logic (both DCMTK and internal)
- Configuration building
- Readiness checking

### Refactoring Proposal

Split into focused modules:

```
adapters/dimse/
├── mod.rs              # ProtocolAdapter implementation
├── scp_manager.rs      # SCP lifecycle, registry, startup
├── config_builder.rs   # Build DimseConfig from options
└── readiness.rs        # TCP connection readiness checks
```

**Benefits**:
- ✅ Clearer separation of concerns
- ✅ Easier to test SCP startup logic independently
- ✅ Better organization

**Priority**: 🟡 Medium  
**Effort**: Low (1 day)

---

## 6. Error Handling Improvements

### Issue

Some error messages are generic and lack context:

```rust
// Current:
DimseError::operation_failed("C-FIND query failed")

// Could be:
DimseError::operation_failed(format!(
    "C-FIND query failed: level={}, params={:?}, error={}",
    query_level, parameters, e
))
```

### Refactoring Proposal

Add context-aware error constructors:

```rust
impl DimseError {
    pub fn association_with_context(
        msg: impl Into<String>,
        peer: &str,
        aet: &str
    ) -> Self {
        Self::AssociationRejected(format!(
            "{} (peer: {}, AET: {})",
            msg.into(), peer, aet
        ))
    }
    
    pub fn parse_with_context(
        msg: impl Into<String>,
        bytes: usize,
        offset: usize
    ) -> Self {
        Self::DicomParsing(format!(
            "{} (at offset {} of {} bytes)",
            msg.into(), offset, bytes
        ))
    }
    
    pub fn query_failed(
        operation: &str,
        level: &str,
        params: &HashMap<String, String>,
        error: &dyn std::fmt::Display,
    ) -> Self {
        Self::OperationFailed(format!(
            "{} query failed: level={}, params={:?}, error={}",
            operation, level, params, error
        ))
    }
}
```

**Priority**: 🟡 Medium  
**Effort**: Low (0.5 days)

---

## 7. Configuration Organization

### Issue

`DimseConfig` has many fields (20+) that could be better organized:

```rust
pub struct DimseConfig {
    pub local_aet: String,
    pub bind_addr: IpAddr,
    pub port: u16,
    pub incoming_store_port: u16,
    pub max_pdu: u32,
    pub connect_timeout_ms: u64,
    pub association_timeout_ms: u64,
    pub storage_dir: PathBuf,
    pub tls: Option<TlsConfig>,
    pub preferred_transfer_syntaxes: Vec<String>,
    pub max_associations: u32,
    pub enable_echo: bool,
    pub enable_find: bool,
    pub enable_move: bool,
    pub enable_get: bool,
    pub enable_store: bool,
    pub external_store_scp: bool,
}
```

### Refactoring Proposal

Group related fields:

```rust
pub struct LocalConfig {
    pub ae_title: String,
    pub bind_addr: IpAddr,
    pub port: u16,
    pub incoming_store_port: u16,
}

pub struct NetworkConfig {
    pub max_pdu: u32,
    pub connect_timeout_ms: u64,
    pub association_timeout_ms: u64,
    pub max_associations: u32,
    pub preferred_transfer_syntaxes: Vec<String>,
}

pub struct StorageConfig {
    pub storage_dir: PathBuf,
    pub external_store_scp: bool,
}

pub struct ServiceConfig {
    pub enable_echo: bool,
    pub enable_find: bool,
    pub enable_move: bool,
    pub enable_get: bool,
    pub enable_store: bool,
}

pub struct DimseConfig {
    pub local: LocalConfig,
    pub network: NetworkConfig,
    pub storage: StorageConfig,
    pub services: ServiceConfig,
    pub tls: Option<TlsConfig>,
}
```

**Priority**: 🟢 Low  
**Effort**: Low-Medium (1 day)

---

## 8. Command Handler Pattern

### Issue

The `dispatch_command()` method uses a large match statement, and each handler is a long method. This could benefit from a command handler pattern.

### Refactoring Proposal

```rust
// scp/commands/trait.rs
#[async_trait]
trait CommandHandler: Send + Sync {
    async fn handle(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        message_id: u16,
        identifier_data: Vec<u8>,
        presentation_context_id: u8,
        config: &DimseConfig,
        query_provider: &Arc<dyn QueryProvider>,
    ) -> Result<()>;
    
    fn command_field(&self) -> u16;
}

// scp/commands/registry.rs
struct CommandRegistry {
    handlers: HashMap<u16, Box<dyn CommandHandler>>,
}

impl CommandRegistry {
    fn new() -> Self {
        let mut handlers = HashMap::new();
        handlers.insert(0x0030, Box::new(EchoHandler));
        handlers.insert(0x0020, Box::new(FindHandler));
        handlers.insert(0x0021, Box::new(MoveHandler));
        handlers.insert(0x0010, Box::new(GetHandler));
        handlers.insert(0x0001, Box::new(StoreHandler));
        Self { handlers }
    }
    
    async fn dispatch(
        &self,
        command_field: u16,
        // ... other params
    ) -> Result<()> {
        let handler = self.handlers.get(&command_field)
            .ok_or_else(|| DimseError::operation_failed(
                format!("Unsupported command: 0x{:04X}", command_field)
            ))?;
        handler.handle(/* ... */).await
    }
}
```

**Priority**: 🟢 Low  
**Effort**: Medium (2-3 days)

---

## 9. Query Parameter Extraction Duplication

### Issue

The `handle_c_find()` method hardcodes query tag extraction. This logic could be reused for C-MOVE and C-GET.

### Refactoring Proposal

```rust
// scp/query_extractor.rs
pub struct QueryExtractor;

impl QueryExtractor {
    /// Common DICOM query tags used in C-FIND, C-MOVE, C-GET
    const QUERY_TAGS: &'static [(&'static str, &'static str)] = &[
        ("PatientID", "00100020"),
        ("PatientName", "00100010"),
        ("StudyInstanceUID", "0020000D"),
        ("SeriesInstanceUID", "0020000E"),
        ("SOPInstanceUID", "00080018"),
        ("StudyDate", "00080020"),
        ("StudyTime", "00080030"),
        ("Modality", "00080060"),
        ("AccessionNumber", "00080050"),
    ];
    
    pub fn extract_common_tags(
        identifier: &InMemDicomObject<StandardDataDictionary>,
    ) -> HashMap<String, String> {
        let mut params = HashMap::new();
        
        for (name, _tag) in Self::QUERY_TAGS {
            if let Ok(elem) = identifier.element_by_name(name) {
                if let Ok(value) = elem.to_str() {
                    if !value.is_empty() {
                        params.insert(name.to_string(), value.to_string());
                    }
                }
            }
        }
        
        params
    }
    
    pub fn extract_query_level(
        identifier: &InMemDicomObject<StandardDataDictionary>,
    ) -> QueryLevel {
        identifier
            .element_by_name("QueryRetrieveLevel")
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|s| s.parse().unwrap_or(QueryLevel::Study))
            .unwrap_or(QueryLevel::Study)
    }
}
```

**Priority**: 🟢 Low  
**Effort**: Low (0.5 days)

---

## 10. Response Building Duplication

### Issue

Each command handler has similar response building code. The response structure is consistent across commands.

### Refactoring Proposal

```rust
// scp/response_builder.rs
pub struct ResponseBuilder;

impl ResponseBuilder {
    pub fn build_command_response(
        command_field: u16,  // Response command field (0x8030, 0x8020, etc.)
        message_id: u16,
        status: u16,
        has_dataset: bool,
        sop_class_uid: &str,
    ) -> InMemDicomObject {
        let mut response = InMemDicomObject::new_empty();
        
        response.put(DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            PrimitiveValue::from(command_field),
        ));
        
        response.put(DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            PrimitiveValue::from(message_id),
        ));
        
        response.put(DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            PrimitiveValue::from(if has_dataset { 0x0000u16 } else { 0x0101u16 }),
        ));
        
        response.put(DataElement::new(
            tags::STATUS,
            VR::US,
            PrimitiveValue::from(status),
        ));
        
        response.put(DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            PrimitiveValue::from(sop_class_uid),
        ));
        
        response
    }
    
    pub async fn encode_and_send(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        response: InMemDicomObject,
        dataset: Option<&DatasetStream>,
        presentation_context_id: u8,
    ) -> Result<()> {
        let ts = TransferSyntaxRegistry
            .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .ok_or_else(|| DimseError::operation_failed("Transfer syntax not found"))?;
        
        let mut response_bytes = Vec::new();
        response.write_dataset_with_ts(&mut response_bytes, ts)
            .map_err(|e| DimseError::operation_failed(format!("Failed to encode: {}", e)))?;
        
        let has_dataset = dataset.is_some();
        let command_pdata = dicom_ul::pdu::PDataValue {
            presentation_context_id,
            value_type: dicom_ul::pdu::PDataValueType::Command,
            is_last: !has_dataset,
            data: response_bytes,
        };
        
        if let Some(ds) = dataset {
            let dicom_obj = ds.to_object().await?;
            let mut identifier_bytes = Vec::new();
            dicom_obj.write_dataset_with_ts(&mut identifier_bytes, ts)
                .map_err(|e| DimseError::operation_failed(format!("Failed to encode dataset: {}", e)))?;
            
            let data_pdata = dicom_ul::pdu::PDataValue {
                presentation_context_id,
                value_type: dicom_ul::pdu::PDataValueType::Data,
                is_last: true,
                data: identifier_bytes,
            };
            
            association.send(&Pdu::PData {
                data: vec![command_pdata, data_pdata],
            }).await
        } else {
            association.send(&Pdu::PData {
                data: vec![command_pdata],
            }).await
        }
        .map_err(|e| DimseError::network(format!("Failed to send response: {}", e)))?;
        
        Ok(())
    }
}
```

**Priority**: 🟡 Medium  
**Effort**: Low (1 day)

---

## Priority Summary

### High Priority 🔴
1. **Split SCP module** (scp.rs → multiple modules)
   - **Effort**: 3-4 days
   - **Impact**: High - improves maintainability significantly
   
2. **Extract DCMTK command building** (reduce duplication)
   - **Effort**: 1-2 days
   - **Impact**: Medium-High - reduces maintenance burden
   
3. **Complete DatasetStream conversions** (fix TODOs)
   - **Effort**: 1 day
   - **Impact**: High - fixes broken functionality

### Medium Priority 🟡
4. **Refactor DimseAdapter module structure**
   - **Effort**: 1 day
   - **Impact**: Medium - improves organization
   
5. **Improve error context** (add context to errors)
   - **Effort**: 0.5 days
   - **Impact**: Low-Medium - better debugging
   
6. **Extract response building helpers**
   - **Effort**: 1 day
   - **Impact**: Medium - reduces duplication

### Low Priority 🟢
7. **Reorganize configuration structs**
   - **Effort**: 1 day
   - **Impact**: Low - cosmetic improvement
   
8. **Implement command handler pattern**
   - **Effort**: 2-3 days
   - **Impact**: Low - architectural improvement
   
9. **Extract query parameter logic**
   - **Effort**: 0.5 days
   - **Impact**: Low - minor code reuse

---

## Additional Observations

### Dead Code
- `handle_dimse_request()` and `send_response()` in SCP are marked `#[allow(dead_code)]`
- Router abstraction exists but isn't integrated with SCP
- Consider removing or documenting as future work

### Incomplete Features
- C-MOVE handler returns "Unable to perform sub-operations" (status 0xA702)
- C-GET handler returns "Unable to perform sub-operations" (status 0xA702)
- These are documented as stubs but should be prioritized if needed

### Testing Gaps
- Command handlers could benefit from more unit tests
- Response building logic could be tested independently
- Integration tests for end-to-end DIMSE operations

### Documentation Needs
- Complex PDU accumulation logic in `handle_pdata()` needs more comments
- Command field values (0x0030, 0x0020, etc.) should be documented or use constants
- Transfer syntax selection logic could use explanation

---

## Implementation Roadmap

### Phase 1: Critical Fixes (1 week)
1. Complete DatasetStream conversions
2. Extract DCMTK command building
3. Improve error context

### Phase 2: Module Organization (1 week)
1. Split SCP module into sub-modules
2. Refactor DimseAdapter structure
3. Extract response building helpers

### Phase 3: Architectural Improvements (1-2 weeks)
1. Integrate router abstraction (if desired)
2. Implement command handler pattern
3. Extract query parameter logic
4. Reorganize configuration

### Phase 4: Completeness (ongoing)
1. Implement C-MOVE fully
2. Implement C-GET fully
3. Add comprehensive tests
4. Improve documentation

---

## Notes

- All refactorings should maintain backward compatibility with existing API
- Consider creating migration guide if public API changes
- Run tests after each refactoring phase
- Consider incremental refactoring to avoid large merge conflicts
```

Create the file `./dev/analysis/dimse-refactoring.md` with this content. It includes:
- Executive summary
- 10 refactoring opportunities with examples
- Priority levels
- Implementation roadmap
- Additional observations

Should I adjust anything or add more detail to any section?
