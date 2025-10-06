# DICOM C-FIND to JSON Conversion - Architecture Summary

## Context
Building a Rust-based DICOM proxy that converts DIMSE C-FIND requests/responses bidirectionally with JSON. Starting with Patient Root Query/Retrieve, but needs to handle any DICOM attributes (general model).

## DIMSE C-FIND Structure

A C-FIND request consists of:

1. **DIMSE Command Set** (Group 0000)
    - Message ID
    - Affected SOP Class UID (e.g., Patient Root Query/Retrieve)
    - Priority (LOW/MEDIUM/HIGH)
    - Data Set Type

2. **Identifier Dataset**
    - Query keys with values, wildcards, or empty (universal match)
    - Each attribute: Group, Element, VR, Value
    - Return keys (empty values indicating what to return)

## dicom-rs Capabilities

- **Version 0.6.0+** includes DICOM JSON support
- Implements **DICOM JSON Model (Part 18 Section F)** standard
- `dicom-dump --format json` outputs standard DICOM JSON
- `json` crate handles serialization/deserialization of datasets

## Standard DICOM JSON Format (Part 18)

```json
{
  "00100020": {
    "vr": "LO",
    "Value": ["12345"]
  },
  "00100010": {
    "vr": "PN",
    "Value": [{"Alphabetic": "DOE^JOHN"}]
  }
}
```

**Characteristics:**
- Tag keys: Uppercase hex without separators `"GGGGEEEE"`
- VR field: Explicit Value Representation
- Value: Always an array, even for single values
- Special structures: PN uses objects with Alphabetic/Ideographic/Phonetic

## Recommended JSON Structure

**Leverage dicom-rs standard format, add command metadata:**

```json
{
  "command": {
    "message_id": 1,
    "sop_class_uid": "1.2.840.10008.5.1.4.1.2.1.1",
    "priority": "MEDIUM",
    "direction": "REQUEST"
  },
  "identifier": {
    "00100020": {
      "vr": "LO",
      "Value": ["12345"]
    },
    "00100010": {
      "vr": "PN",
      "Value": [{"Alphabetic": "DOE*"}]
    },
    "00080020": {
      "vr": "DA",
      "Value": ["20240101-20241231"]
    },
    "00080050": {
      "vr": "SH",
      "Value": []
    }
  },
  "query_metadata": {
    "00100010": {"match_type": "WILDCARD"},
    "00080020": {"match_type": "RANGE"},
    "00080050": {"match_type": "RETURN_KEY"}
  }
}
```

## Query Matching Types

1. **EXACT**: Single value matching `"Value": ["12345"]`
2. **WILDCARD**: Pattern with `*` (zero/more) or `?` (single char) - `"Value": ["DOE*"]`
3. **RANGE**: Date/time ranges - `"Value": ["20240101-20241231"]`
4. **LIST**: Multiple values - `"Value": ["A", "B", "C"]`
5. **RETURN_KEY**: Empty array - `"Value": []`
6. **UNIVERSAL**: Zero-length query matching any value
7. **SEQUENCE**: Nested structures with recursive items

## Conversion Logic

### JSON → DIMSE C-FIND

1. Parse command section → Build DIMSE command dataset
2. For each identifier attribute:
    - Lookup/validate VR from data dictionary
    - Convert query type to DICOM value:
        - EXACT → single value
        - WILDCARD → preserve `*`, `?` in string
        - RANGE → format as `from-to` per VR rules
        - RETURN_KEY → empty value
        - LIST → multi-valued attribute (VM > 1)
        - SEQUENCE → recursively encode items
3. Encode to DICOM binary using proper VR encoding

### DIMSE C-FIND → JSON

1. Parse DIMSE command → Extract to command section
2. For each attribute in identifier:
    - Extract tag, VR, value using dicom-rs
    - Infer query type:
        - Empty value → RETURN_KEY
        - Contains `*` or `?` → WILDCARD
        - Contains `-` and VR is DA/TM/DT → check if valid RANGE
        - Multi-value (VM > 1) → LIST
        - Sequence → SEQUENCE
        - Otherwise → EXACT
    - Serialize using dicom-rs JSON format
3. Optionally add query_metadata for explicit type annotations

## Sequences Handling

```json
"00400275": {
  "vr": "SQ",
  "Value": [{
    "00321060": {
      "vr": "LO",
      "Value": ["*CT*"]
    }
  }]
}
```

## Edge Cases

1. **Private tags**: Handle as UN (Unknown) VR or raw bytes
2. **VR validation**: Validate against data dictionary
3. **Empty strings vs null**: Both can mean return key
4. **Case sensitivity**: PN is case-insensitive, LO can be case-sensitive
5. **Character sets**: Specific Character Set (0008,0005) affects encoding
6. **Trailing spaces**: DICOM pads strings - handle consistently
7. **Date/Time formats**: DICOM uses YYYYMMDD, may want ISO8601 in JSON

## Architecture Benefits

- **Reuses** dicom-rs standard JSON serialization for identifiers
- **Standards-compliant** with DICOM Part 18
- **Extends** minimally for command metadata and query semantics
- **Bidirectional** conversion with query intent preservation
- **General model** handles any DICOM attribute through VR-based encoding

## Open Design Questions

1. **Query type inference**: Auto-detect wildcards or require explicit `match_type`?
2. **VR handling**: Optional (dictionary lookup) or required (validated)?
3. **Tag representation**: Prefer `"GGGGEEEE"` (standard), `"(GGGG,EEEE)"`, or semantic names?
4. **Response handling**: C-FIND returns multiple datasets - array of identifiers?
5. **Error representation**: How to encode DIMSE status codes in JSON responses?