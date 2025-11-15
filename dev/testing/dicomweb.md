# DICOMWeb Testing Guide

This guide covers all DICOMweb endpoint paths supported by Harmony proxy, organized by QIDO-RS and WADO-RS services.

## Prerequisites

- Harmony proxy running on port 8081 (or adjust URLs accordingly)
- Backend DICOM server configured (e.g., dcmqrscp, Orthanc, etc.)
- Sample DICOM data loaded in the backend

## QIDO-RS Endpoints (Query/Retrieve Information)

### 1. Query Studies

**All studies:**
```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies"
```

**Studies with query parameters:**
```zsh
# Search by patient name (supports wildcards)
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?PatientName=SMITH*"

# Search by patient ID
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?PatientID=12345"

# Search by study date
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?StudyDate=20231015"

# Multiple query parameters
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?PatientName=DOE*&StudyDate=20231015&Modality=CT"

# Using includefield to limit returned attributes
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?PatientName=SMITH*&includefield=PatientName,StudyDate,StudyDescription"
```

**Query specific study:**
```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0"
```

### 2. Query Series

**All series in a study:**
```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series"
```

**Series with query parameters:**
```zsh
# Filter by modality
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series?Modality=CT"

# Filter by series number
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series?SeriesNumber=1"

# With includefield
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series?includefield=Modality,SeriesDescription,SeriesNumber"
```

### 3. Query Instances

**All instances in a series:**
```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.112.0/instances"
```

**Instances with query parameters:**
```zsh
# Filter by instance number
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.112.0/instances?InstanceNumber=1"

# With includefield
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.112.0/instances?includefield=InstanceNumber,SOPInstanceUID"
```

## WADO-RS Endpoints (Web Access to DICOM Objects)

### 1. Metadata Retrieval

**Study metadata:**
```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/metadata"
```

**Series metadata:**
```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.112.0/metadata"
```

**Instance metadata:**
```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.112.0/instances/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.113.0/metadata"
```

**Metadata with includefield:**
```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/metadata?includefield=PatientName,StudyDate,Modality"
```

### 2. Instance Retrieval

**Retrieve DICOM instance (full object):**
```zsh
curl -X GET \
  -H "Accept: application/dicom" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.112.0/instances/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.113.0" \
  --output downloaded_instance.dcm
```

**Retrieve instance as multipart:**
```zsh
curl -X GET \
  -H "Accept: multipart/related; type=application/dicom" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.112.0/instances/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.113.0" \
  --output multipart_response.txt
```

### 3. Frame Retrieval (Rendered Images)

**Single frame as JPEG:**
```zsh
curl -X GET \
  -H "Accept: image/jpeg" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.112.0/instances/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.113.0/frames/1" \
  --output frame1.jpg
```

**Single frame as PNG:**
```zsh
curl -X GET \
  -H "Accept: image/png" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.112.0/instances/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.113.0/frames/1" \
  --output frame1.png
```

**Multiple frames:**
```zsh
curl -X GET \
  -H "Accept: multipart/related; type=image/jpeg" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.112.0/instances/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.113.0/frames/1,2,3" \
  --output multiframe_response.txt
```

### 4. Bulk Data Retrieval

**Retrieve bulk data:**
```zsh
curl -X GET \
  -H "Accept: application/octet-stream" \
  "http://127.0.0.1:8081/dicomweb/bulkdata/some-bulk-data-uri" \
  --output bulkdata.bin
```

## CORS and OPTIONS Requests

**Test CORS preflight:**
```zsh
curl -X OPTIONS \
  -H "Origin: http://localhost:3000" \
  -H "Access-Control-Request-Method: GET" \
  -H "Access-Control-Request-Headers: accept, content-type" \
  "http://127.0.0.1:8081/dicomweb/studies" \
  -v
```

## Pagination

Pagination allows limiting the number of results returned by QIDO-RS queries. Two parameters control pagination:

- **`limit`**: Maximum number of results to return (default: 100)
- **`offset`**: Number of results to skip before returning (default: 0)

**Note**: The `offset` parameter is applied in the Harmony middleware layer after receiving results from the DIMSE backend, since DIMSE C-FIND does not natively support offset. The `limit` parameter is passed to the DIMSE backend as `max_results`.

**Paginate studies:**
```zsh
# Get first 10 studies
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?limit=10"
```

**Paginate with offset:**
```zsh
# Skip first 20 studies, return next 10
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?limit=10&offset=20"
```

**Pagination with query filters:**
```zsh
# Search with pagination
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?PatientName=SMITH*&limit=25&offset=0"
```

## Date Range Queries

DICOM date attributes support range queries using the format `YYYYMMDD-YYYYMMDD`. Date ranges are passed directly to the DIMSE backend, which natively supports range matching.

**Study date range:**
```zsh
# Studies from January 2024
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?StudyDate=20240101-20240131"
```

**Single date:**
```zsh
# Studies on a specific date
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?StudyDate=20240115"
```

**Multiple date filters:**
```zsh
# Combine StudyDate and SeriesDate
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?StudyDate=20240101-20240131&SeriesDate=20240115"
```

## Advanced Query Examples

**Complex study search with multiple criteria:**
```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?PatientName=DOE*&StudyDate=20231015-20231020&Modality=CT,MR&StudyDescription=*BRAIN*&includefield=PatientName,PatientID,StudyDate,StudyTime,StudyDescription,Modality"
```

**Paginated search with date range:**
```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies?StudyDate=20240101-20240331&PatientName=DOE*&limit=50&offset=0"
```

**Series search with detailed filtering:**
```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8081/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series?Modality=CT&BodyPartExamined=HEAD&SeriesDescription=*AXIAL*&includefield=SeriesInstanceUID,SeriesNumber,SeriesDescription,Modality,BodyPartExamined"
```

## Response Expectations

- **QIDO-RS responses**: `application/dicom+json` with HTTP 200 (results) or 204 (no results)
- **WADO-RS metadata**: `application/dicom+json` with HTTP 200
- **WADO-RS instances**: `application/dicom` or `multipart/related` with HTTP 200
- **WADO-RS frames**: `image/jpeg`, `image/png`, or `multipart/related` with HTTP 200
- **Errors**: `application/json` with appropriate HTTP error codes (400, 406, 500, etc.)

## Common DICOM Attributes for Query Parameters

- `PatientName`, `PatientID`, `PatientBirthDate`
- `StudyInstanceUID`, `StudyDate`, `StudyTime`, `StudyDescription`, `AccessionNumber`
- `SeriesInstanceUID`, `SeriesNumber`, `SeriesDescription`, `Modality`
- `SOPInstanceUID`, `InstanceNumber`
- `BodyPartExamined`, `InstitutionName`, `ReferringPhysicianName`

## Troubleshooting

- **502 Bad Gateway**: Backend DICOM server not responding
- **204 No Content**: Query executed successfully but no matching results
- **406 Not Acceptable**: Frame decoding error (unsupported transfer syntax)
- **501 Not Implemented**: Endpoint not configured with dicomweb_bridge middleware
