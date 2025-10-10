# DICOM Testing with Orthanc

This guide walks you through setting up Orthanc with Docker for DICOM testing.

## 1. Create a working directory

```bash
mkdir orthanc-docker
cd orthanc-docker
```

## 2. Create a configuration file

Save the following as `orthanc.json` in the same directory:

```json
{
  "Name": "OrthancDocker",
  "HttpServerEnabled": true,
  "HttpPort": 8042,
  
  "DicomServerEnabled": true,
  "DicomAet": "ORTHANC",
  "DicomPort": 4242,
  
  "StrictAetComparison": false,
  "DicomCheckCalledAet": false,
  "DicomCheckModalityHost": false,
  
  "DicomModalities": {
    "HARMONY_SCU": {
      "AET": "HARMONY_SCU",
      "Host": "localhost",
      "Port": 11112,
      "Manufacturer": "Generic",
      "AllowEcho": true,
      "AllowFind": true,
      "AllowStore": true,
      "AllowMove": true
    }
  }
}
```

### Configuration Details

This configuration:

- **Enables** the DICOM server and HTTP UI
- **Disables** strict AE and host checks
- **Authorizes** a modality called `HARMONY_SCU` to perform C-FIND, C-STORE, etc.

## 3. Stop and remove any existing container (optional but recommended)

```bash
docker stop orthanc || true
docker rm orthanc || true
```

> **Note:** The `|| true` part ignores errors if the container doesn't exist.

## 4. Start Orthanc with Docker

```bash
docker run -d --name orthanc \
  -p 8042:8042 -p 4242:4242 \
  -v "$(pwd)/orthanc.json:/etc/orthanc/orthanc.json:ro" \
  orthancteam/orthanc:latest
```

## 5. Verify that Orthanc is running

```bash
docker ps
```

You should see a container named `orthanc` with ports 8042 and 4242 mapped.

### Check the Web UI

Open your browser and navigate to:

**URL:** http://localhost:8042

**Default login:** `orthanc` / `orthanc`

## 6. Upload a DICOM study (optional for testing)

If you have a `.dcm` file:

```bash
storescu -aec ORTHANC 127.0.0.1 4242 path/to/file.dcm
```

## 7. Verify connectivity with C-ECHO

```bash
echoscu -aet HARMONY_SCU -aec ORTHANC 127.0.0.1 4242
```

You should see **"Received Echo Response (Success)"**.

## 8. Query with C-FIND

```bash
findscu -S -aet HARMONY_SCU -aec ORTHANC \
  -k 0008,0052=STUDY \
  -k 0020,000D \
  -k 0008,0020 \
  -k 0010,0010 \
  -k 0010,0020 \
  127.0.0.1 4242 -v
```

If Orthanc sees the study, you'll receive one or more **Pending** responses with metadata.

## 9. Troubleshoot (if needed)

If the query still fails, check Orthanc logs:

```bash
docker logs -f orthanc
```

Look for messages about AE mismatches or missing modalities.

---

## Killing Stuck Processes

### Kill stuck storescp

**Find it:**
```bash
ps aux | grep storescp
```

**Kill it:**
```bash
sudo kill -9 <PID>
```

**Or use pkill:**
```bash
sudo pkill -f storescp
```

### Check port usage

```bash
sudo lsof -i :11112
```
