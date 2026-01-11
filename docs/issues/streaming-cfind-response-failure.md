# Issue: C-FIND Response Not Received by Orthanc Client

## Summary

When executing a C-FIND through the streaming pipeline (Orthanc A → Harmony A → Harmony B → Orthanc B), the query executes successfully and Harmony A logs "C-FIND completed with 1 results", but Orthanc A receives an error:

```
DIMSE: receiveCommand: cmdSet->read() Failed (0001:0004 Invalid stream)
```

## Environment

- Test environment: `docker/streaming-test/`
- Branch: `feat/aura-2198-streaming-dicom`
- Commit: `7769da1`

## Data Flow (All Working)

1. ✅ Orthanc A sends C-FIND to Harmony A (DIMSE)
2. ✅ Harmony A receives request via DIMSE SCP
3. ✅ `dicom_to_http` middleware transforms to HTTP POST with `x-dimse-op: find` header
4. ✅ `http_streaming_backend` forwards to Harmony B
5. ✅ Harmony B receives HTTP request, extracts `x-dimse-op` from header
6. ✅ `dicom_scu_streaming` backend executes C-FIND on Orthanc B
7. ✅ Orthanc B returns 1 result
8. ✅ Harmony B returns JSON array response `[{...}]`
9. ✅ Harmony A parses JSON array (1 element)
10. ✅ Harmony A converts JSON to DICOM dataset (68 bytes)
11. ✅ Harmony A logs: "C-FIND completed with 1 results"
12. ❌ Orthanc A receives "Invalid stream" error

## Observed Behavior

### Harmony A Logs (Success)
```
C-FIND query: level=STUDY, params={"00100020": "*"}
Using streaming pipeline for C-FIND
Parsing JSON array with 1 elements
Streaming pipeline completed: 1 instances, 1 total datasets
Found 1 matching datasets
Encoded dataset: 68 bytes
C-FIND completed with 1 results
Association release requested by 192.168.80.2:xxxxx
Error handling association: Network error: Failed to send release: failed to send PDU message on wire
```

### Orthanc A Error
```json
{
  "Details": "DicomAssociation - C-FIND to AET \"HARMONY_A_SCU\": DIMSE: receiveCommand: cmdSet->read() Failed (0001:0004 Invalid stream)",
  "HttpStatus": 500
}
```

## Hypothesis

The issue appears to be in the DIMSE response sending phase. Possible causes:

1. **Timing issue**: The pending response (0xFF00) and final response (0x0000) may not be sent before Orthanc times out or closes the connection

2. **PDU encoding issue**: The response PDU format might be incorrect, causing Orthanc to reject it as "Invalid stream"

3. **Association state issue**: The association may be in an unexpected state after the streaming pipeline completes

## Relevant Code

- Response sending: `crates/dimse/src/scp/response_builder.rs` - `send_find_response()`
- C-FIND handler: `crates/dimse/src/scp/commands/find.rs` - `handle_c_find()`
- Query provider: `src/adapters/dimse/query_provider.rs` - `run_streaming()`

## To Reproduce

1. Start the streaming test environment:
   ```bash
   cd docker/streaming-test
   docker compose up -d
   ```

2. Upload a study to Orthanc B with a PatientID

3. Run C-FIND from Orthanc A:
   ```bash
   curl -u orthanc:orthanc -X POST http://localhost:18042/modalities/HARMONY_A_SCU/query \
     -d '{"Level":"Study","Query":{"PatientID":"*"}}'
   ```

4. Observe the error response and check logs:
   ```bash
   docker logs harmony-a 2>&1 | tail -50
   ```

## Notes

- The basic C-FIND SCP test (`test_scp_accepts_c_find`) passes
- The issue is specific to the streaming pipeline path
- Non-streaming C-FIND (direct DIMSE backend) may work correctly
