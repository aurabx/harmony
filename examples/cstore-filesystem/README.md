# DICOM C-STORE Filesystem Example

This example demonstrates how to configure Harmony to receive DICOM files via the C-STORE protocol and store them using the local filesystem backend.

## Structure

```
cstore-filesystem/
├── config.toml          # Main configuration (storage, network)
├── pipelines/
│   └── main.toml       # Pipeline definition (endpoints, logic)
└── README.md           # This file
```

## Configuration

The configuration sets up:
1. A **Filesystem Storage** backend rooted at `./data`.
2. A **DICOM SCP** endpoint on port 11112 that accepts C-STORE.
3. A **Storage Backend** service that archives the received files to `./data/archive`.

## Usage

1. Start the server from the project root:
   ```bash
   cargo run -- --config examples/cstore-filesystem/config.toml
   ```

2. Send a DICOM file using `storescu` (from DCMTK):
   ```bash
   storescu -v -aet TEST_SCU -aec HARMONY_SCP localhost 11112 path/to/test.dcm
   ```

3. Verify the file was stored:
   ```bash
   ls -R examples/cstore-filesystem/data/dimse/
   ```

## How it Works

When a C-STORE request is received:
1. The file is streamed to disk at `{storage.path}/dimse/<uuid>.dcm` (temporary holding).
2. The `dicom_ingest` pipeline is executed with the file path in the payload.
3. The `filesystem_storage` backend receives the request and copies/moves the file to `./data/archive` (or simply acknowledges it if configured as such).
4. A success response is returned to the sender.
