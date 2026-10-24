---
title: Supported Content Types
description: How each supported content type is parsed into normalised data.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/content-types.md
---

# Supported Content Types

How each supported content type is parsed into normalised data. This page is part of [Multi-Content-Type Support](content-types.md).

## Supported Content Types

### 1. JSON (Default)

**Content-Type Headers:**
- `application/json`
- `application/fhir+json` (FHIR resources)
- `application/dicom+json` (DICOM JSON)

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: application/json" \
  -d '{"name": "Alice", "age": 30}'
```

**Normalized Structure:**
```json
{
  "name": "Alice",
  "age": 30
}
```

**Notes:**
- Default format when Content-Type is missing or unrecognized
- Direct pass-through to normalized_data
- Validates JSON syntax

### 2. XML

**Content-Type Headers:**
- `application/xml`
- `text/xml`
- `application/soap+xml`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: application/xml" \
  -d '<person><name>Bob</name><age>25</age></person>'
```

**Normalized Structure:**
```json
{
  "person": {
    "name": "Bob",
    "age": "25"
  }
}
```

**XML Features:**
- **Text-only elements**: Converted to simple string values
- **Attributes**: Prefixed with `@` (e.g., `"@id": "123"`)
- **Nested elements**: Preserved as nested objects
- **Multiple elements with same name**: Converted to arrays
- **Mixed content**: Text stored in `#text` field when attributes present

**Example with Attributes:**
```xml
<person id="123" type="customer">
  <name>Charlie</name>
</person>
```

Becomes:
```json
{
  "person": {
    "@id": "123",
    "@type": "customer",
    "name": "Charlie"
  }
}
```

**Security:** XXE (XML External Entity) attacks are prevented - quick-xml does not support external entities by default.

### 3. CSV

**Content-Type Header:**
- `text/csv`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: text/csv" \
  -d 'name,age,city
Alice,30,NYC
Bob,25,LA'
```

**Normalized Structure:**
```json
{
  "rows": [
    {"name": "Alice", "age": "30", "city": "NYC"},
    {"name": "Bob", "age": "25", "city": "LA"}
  ],
  "row_count": 2
}
```

**CSV Features:**
- First row treated as header
- All values parsed as strings
- Empty fields supported
- Handles quoted fields with commas

**Security:** Formula injection prevention - fields starting with `=`, `+`, `-`, or `@` are automatically prefixed with a single quote (`'`) to prevent execution in spreadsheet applications.

Example:
```csv
name,formula
Alice,=SUM(A1:A10)
```

Becomes:
```json
{
  "rows": [
    {"name": "Alice", "formula": "'=SUM(A1:A10)"}
  ]
}
```

### 4. Form URL-Encoded

**Content-Type Header:**
- `application/x-www-form-urlencoded`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d 'name=Alice&age=30&city=NYC'
```

**Normalized Structure:**
```json
{
  "name": "Alice",
  "age": "30",
  "city": "NYC"
}
```

**Array Support:**
Use `[]` notation for arrays:
```bash
curl -X POST http://localhost:8080/api/data \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d 'name=Alice&interests[]=coding&interests[]=music'
```

Becomes:
```json
{
  "name": "Alice",
  "interests": ["coding", "music"]
}
```

### 5. Multipart Form Data

**Content-Type Header:**
- `multipart/form-data; boundary=<boundary>`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/upload \
  -F "name=Alice" \
  -F "age=30" \
  -F "file=@document.pdf"
```

**Normalized Structure:**
```json
{
  "fields": {
    "name": "Alice",
    "age": "30"
  },
  "files": [
    {
      "name": "file",
      "filename": "document.pdf",
      "content_type": "application/pdf",
      "size": 12345,
      "checksum": "a1b2c3d4..."
    }
  ]
}
```

**File Handling:**
- Files are NOT saved to disk automatically
- File metadata captured for pipeline processing
- SHA256 checksum computed for integrity verification
- Middleware/backends can access file data via envelope

### 6. Binary Content

**Content-Type Headers:**
- `image/*` (JPEG, PNG, GIF, etc.)
- `video/*`
- `audio/*`
- `application/pdf`
- `application/zip`
- `application/octet-stream`

**Example Request:**
```bash
curl -X POST http://localhost:8080/api/upload \
  -H "Content-Type: image/jpeg" \
  --data-binary @photo.jpg
```

**Normalized Structure:**
```json
{
  "format": "binary",
  "content_type": "image/jpeg",
  "size": 45678,
  "checksum": "abc123..."
}
```

**Notes:**
- Binary data preserved in `original_data` field of envelope
- Checksum allows integrity verification
- Middleware can process binary data directly
