# HTTP File Upload Example

This example demonstrates how to route HTTP requests with binary payloads directly to the storage backend (local filesystem).

## Configuration

The configuration defines:
1. An HTTP endpoint (`/upload`) receiving POST requests.
2. A storage backend writing to `./tmp/uploads`.
3. A pipeline connecting the two.

**Backend Configuration:**
```toml
[backends.storage_backend]
service = "storage"
[backends.storage_backend.options]
root = "./tmp/uploads"
write_pattern = "files/{uuid}.bin"
```

The `write_pattern` uses `{uuid}` to automatically generate a unique filename for each upload. The storage backend also supports `{timestamp}` and metadata replacement (e.g. `{tenant}` if using JWT auth).

## Running the Demo

Run the automated demo script:

```bash
./examples/http-file-upload/demo.sh
```

## Manual Testing

1. Start Harmony with the example config:
   ```bash
   cargo run -- --config examples/http-file-upload/config.toml
   ```

2. Upload a file using curl:
   ```bash
   curl -X POST http://127.0.0.1:8080/upload --data-binary @path/to/your/file.txt
   ```

3. Check the response. It should look like:
   ```json
   {
     "location": "./tmp/uploads/files/550e8400-e29b-41d4-a716-446655440000.bin",
     "path": "files/550e8400-e29b-41d4-a716-446655440000.bin",
     "status": "stored"
   }
   ```
