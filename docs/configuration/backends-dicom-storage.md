---
title: DICOM, Storage, and Echo Backends
description: Configuration for the DICOM SCU, storage, and echo backends.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/backends.md
---

# DICOM, Storage, and Echo Backends

Configuration for the DICOM SCU, storage, and echo backends. This page is part of [Backends](backends.md).

## DICOM SCU (Service Class User)

A DICOM DIMSE backend for connecting to remote DICOM PACS via C-ECHO/C-FIND/C-MOVE/C-GET/C-STORE operations.

**Service behavior**:
- Converts `RequestEnvelope` to DICOM DIMSE operations (SCU - outgoing requests)
- Uses `resolve_operation()` to determine the DIMSE action: Query→C-FIND, Retrieve→C-GET/C-MOVE, Store→C-STORE, Ping→C-ECHO. Falls back to path-based detection if no Command is set. See [commands.md](../architecture/commands.md).
- Communicates with remote DICOM nodes using AE titles
- Converts DICOM responses back to `ResponseEnvelope`
- Supports C-ECHO, C-FIND, C-MOVE, C-GET, C-STORE operations

**Configuration**:
```toml
[backends.<name>]
service = "dicom_scu"
[backends.<name>.options]
aet = "REMOTE_AET"           # Remote Application Entity Title
host = "pacs.example.com"
port = 4242
local_aet = "HARMONY_SCU"    # Local AE title
dimse_retrieve_mode = "get" # DICOM retrieval mode: "get" or "move"
use_tls = false
``` path=null start=null

**Configuration Options**:
- `aet` (string, required): Remote Application Entity Title
- `host` (string, required): PACS hostname or IP address
- `port` (integer, required): PACS port number
- `local_aet` (string, optional): Local AE title (default: "HARMONY_SCU")
- `dimse_retrieve_mode` (string, optional): DICOM retrieval mode (default: "get")
  - `"get"` (C-GET): Direct image retrieval, works without PACS-side AE configuration
  - `"move"` (C-MOVE): Requires PACS to know SCU's AE title and network address
- `use_tls` (boolean, optional): Enable TLS encryption (default: false)
- `incoming_store_port` (integer, optional): Port for C-STORE SCP when using C-MOVE
- `persistent_store_scp` (boolean, optional): Keep persistent C-STORE SCP listening

**Example**: DICOM PACS backend
```toml
[backends.orthanc_pacs]
service = "dicom_scu"  # Use "dicom_scu" for SCU backend
[backends.orthanc_pacs.options]
aet = "ORTHANC"
host = "localhost"
port = 4242
local_aet = "HARMONY_SCU"
dimse_retrieve_mode = "get"
incoming_store_port = 11112
persistent_store_scp = true
use_tls = false
``` path=null start=null

**Prerequisites**: DCMTK is needed for C-MOVE only (see [DIMSE Integration](../integrations/dicom-dimse.md))

**Supported Operations**:
- `C-ECHO`: Test connectivity
- `C-FIND`: Query for studies/series/images
- `C-MOVE`: Request dataset transfer
- `C-GET`: Retrieve datasets
- `C-STORE`: Send one instance to the PACS per request

**C-STORE request body**:

| Body | Sent by | Result |
|------|---------|--------|
| A DICOM Part 10 file | An HTTP client | The instance is sent |
| A JSON object with the Part 10 file as base64 in `part10_b64` | The `dicom_scp` endpoint in the same pipeline | The instance is sent |
| A JSON object without `part10_b64` | | Rejected |
| Any other body that does not start with `{` | An HTTP client | Sent as bare data set bytes, read as Implicit VR Little Endian |
| A body that starts with `{` and is not valid JSON | | Rejected |

The backend never opens a file that a request names. A JSON body with a `file`
key and no `part10_b64` is rejected.

The backend sends the instance in `part10_b64` as it was received. A change
that middleware makes to the `dataset` key does not change what is sent. A
`transform` that removes patient tags from `dataset` does not de-identify the
instance that reaches the PACS.

A middleware that replaces the whole body, such as `transform`, must keep
`part10_b64` in its output. Without it the backend has no instance to send and
the store fails.

The backend reads the transfer syntax of the instance from the file meta group
and proposes it to the PACS:

| Instance | Transfer syntaxes proposed |
|----------|----------------------------|
| Compressed pixel data, for example JPEG Lossless | The transfer syntax of the instance |
| Uncompressed pixel data | The transfer syntax of the instance, then Implicit VR Little Endian |
| Deflated Explicit VR Little Endian | Explicit VR Little Endian, then Implicit VR Little Endian. The instance is sent without deflate compression |
| Bare data set bytes | Implicit VR Little Endian |

The backend does not transcode pixel data. A compressed instance reaches the
PACS in the transfer syntax it was received in. If the PACS does not accept
that transfer syntax, the store fails and nothing is sent. The error names the
SOP class and the transfer syntax.

The backend checks the pixel data against the negotiated transfer syntax
before it sends anything. It refuses an instance whose label and pixel data
disagree: compressed fragments under an uncompressed transfer syntax, or
uncompressed pixel values under a compressed one. The error names the
negotiated transfer syntax and the form of the pixel data. An instance with no
pixel data, such as a structured report, is sent in any transfer syntax.

A `dicom_scp` endpoint in the same pipeline records the transfer syntax it
negotiated with the sending device, so the instance keeps it on the way to the
PACS.

A DICOMweb STOW-RS request reaches this backend as one C-STORE per part. The
`dicomweb` endpoint takes the parts out of the multipart body.

The response is JSON. Read `success` to learn the result:

```json
{"operation":"store","success":true,"remote_aet":"ORTHANC","host":"localhost","port":4242,"sop_class_uid":"1.2.840.10008.5.1.4.1.1.2","sop_instance_uid":"1.2.3"}
```

A failed store has `"success": false` and an `error` string. A `dicom_scp`
endpoint in front of this backend then answers the sending device with status
`0xC000`.

See [DIMSE Integration](../integrations/dicom-dimse.md) for detailed DIMSE usage.

## Storage (Filesystem/S3)

A backend for reading and writing files to local storage or S3-compatible object storage.

**Service behavior**:
- Uses `resolve_operation()` to determine read vs. write: Query/Retrieve→read, Store/Update/Patch→write. Falls back to HTTP method if no Command is set. See [commands.md](../architecture/commands.md).
- **Read** (GET or Query/Retrieve): Reads file at path resolved from `read_pattern`
- **Write** (POST/PUT or Store/Update): Writes request body to path resolved from `write_pattern`
- Supports path templating with metadata, UUIDs, and timestamps
- Returns `Location` header and JSON status on successful write
- `content = "dicom_part10"` writes the DICOM Part 10 instance of a C-STORE instead of the request body (see [Storing received C-STORE instances](#storing-received-c-store-instances))

**Configuration**:
```toml
[backends.<name>]
service = "storage"
[backends.<name>.options]
root = "./data"  # Local path or "s3://bucket/prefix"
read_pattern = "files/{uuid}.bin"
write_pattern = "files/{uuid}.bin"
``` path=null start=null

**S3 Configuration**:
To use S3, set `root` to an S3 URI (`s3://bucket-name/prefix`) and provide credentials:
```toml
[backends.s3_storage]
service = "storage"
[backends.s3_storage.options]
root = "s3://my-bucket/uploads"
region = "us-east-1"
access_key_id = "..."      # Optional (can use env vars)
secret_access_key = "..."  # Optional (can use env vars)
endpoint = "..."           # Optional (for MinIO/custom S3)
``` path=null start=null

**Path Templating**:
Patterns can use placeholders replaced at runtime:
- `{uuid}`: Generates a random UUID (v4)
- `{timestamp}`: Current timestamp (RFC3339)
- `{metadata_key}`: Replaced by value from request metadata (e.g. `{tenant}` from JWT)
- `{field_name}`: Replaced by value from normalized data (e.g. `{PatientID}`)

**Placeholder rules for `write_pattern`**:
- Each placeholder is filled once. A value is not scanned for further placeholders.
- `{uuid}` and `{timestamp}` are always generated. Request metadata or data with those names is ignored.
- A value fills part of one path segment. A write fails when a value is empty, is `.` or `..`, or holds `/` or `\`.
- A write fails when a placeholder has no value in the request.
- `read_pattern` does not follow these rules.

**Example**:
```toml
[backends.patient_docs]
service = "storage"
[backends.patient_docs.options]
root = "./patient_data"
# Write to: ./patient_data/{tenant}/{PatientID}/{uuid}.dcm
write_pattern = "{tenant}/{PatientID}/{uuid}.dcm"
``` path=null start=null

### Storing received C-STORE instances

Put a `storage` backend with `content = "dicom_part10"` behind a `dicom_scp` endpoint to keep each instance a device sends.

```toml
[pipelines.archive]
networks = ["default"]
endpoints = ["scp"]
middleware = ["site_policy"]
backends = ["dicom_archive"]

[backends.dicom_archive]
service = "storage"
[backends.dicom_archive.options]
root = "./data/archive"
content = "dicom_part10"
write_pattern = "{StudyInstanceUID}/{SeriesInstanceUID}/{SOPInstanceUID}.dcm"
``` path=null start=null

- The backend runs after the pipeline's middleware. The backend does not write a C-STORE that `policies` denies.
- The endpoint also keeps a working copy of each instance under `[storage]/dimse` while the pipeline runs.
- The backend must be the pipeline's first backend. Otherwise the endpoint does not hand it the instance, and every store fails.
- The endpoint hands the backend its working copy of the instance inside the process. Middleware cannot see or change it, and a request cannot name it. The backend never reads the `file` path in the body.
- The backend uploads the instance in 10 MiB parts, at most two in flight. One store holds about 30 MiB, whatever the instance size, so large instances such as tomosynthesis fit. It uploads to a temporary name next to the final path (`<name>.partial-<uuid>`), copies it to the final path, then deletes the temporary name.
- If the request is cancelled mid-store, a `.partial-` object or an incomplete S3 multipart upload can be left behind. On S3, add a lifecycle rule that removes incomplete multipart uploads and old `.partial-` objects.
- A middleware that replaces the request body, for example with a changed Part 10 file in `body_b64`, takes effect. The backend then writes the new body and holds it in memory, as over HTTP.
- Over HTTP, the backend takes the Part 10 file as the request body, or as base64 in `part10_b64` of a JSON body. That path holds the whole instance in memory.
- `StudyInstanceUID`, `SeriesInstanceUID`, `SOPInstanceUID` and `PatientID` come from the instance. A value that breaks the placeholder rules fails the store. The device then gets status `0xC000` and keeps its copy.
- A body with no Part 10 instance fails the write.
- A resend of the same bytes counts as stored. Different bytes at an existing path fail the write, so a peer cannot replace an archived instance by reusing its UIDs.
- On the local filesystem the final copy is atomic. On S3 it is a check followed by a copy, because Harmony does not configure S3 conditional copy. Two different instances with the same path that arrive at the same moment can both pass the check, and the second replaces the first.
- Error messages do not include the write path, which may hold patient data.
- An unknown `content` value fails every write.

## Echo (Test)

A simple echo backend that reflects the request back as the response.

**Service behavior**:
- Returns the request envelope as the response
- Useful for testing and debugging pipelines
- No external communication

**Configuration**:
```toml
[backends.<name>]
service = "echo"
``` path=null start=null

**Example**: Echo test backend
```toml
[backends.test_echo]
service = "echo"
``` path=null start=null
