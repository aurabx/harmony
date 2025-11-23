# Harmony DSL v1.9 Migration Guide

**Date**: 2025-11-23  
**Version**: 1.9.0

## Overview

Harmony DSL v1.9 introduces normalized connection settings and reference capabilities (`peer_ref`, `target_ref`). This allows you to define connection details once in a peer or target and reuse them across multiple endpoints or backends.

Existing configurations remain fully backward compatible. Migration to the new format is optional but recommended for cleaner and more maintainable configurations.

## New Features

### 1. Configuration References

You can now define `peers` and `targets` in your main `config.toml` and reference them in your pipeline definitions.

**Before:**
Duplicated connection settings in every backend.

```toml
# pipelines/main.toml
[backends.api_1]
service = "http"
[backends.api_1.options]
base_url = "https://api.example.com"

[backends.api_2]
service = "http"
[backends.api_2.options]
base_url = "https://api.example.com"
```

**After:**
Define once, reference many times.

```toml
# config.toml
[targets.prod_api]
connection.host = "api.example.com"
connection.protocol = "https"
timeout_secs = 60

# pipelines/main.toml
[backends.api_1]
service = "http"
target_ref = "prod_api"

[backends.api_2]
service = "http"
target_ref = "prod_api"
```

### 2. Normalized Connection Settings

A consistent structure for connection settings is now available for all components:

```toml
connection.host = "hostname"
connection.port = 8080
connection.protocol = "http"  # Replaces 'type' (backward compatible)
connection.base_path = "/api/v1"
```

## Migration Examples

### HTTP Backend

**Legacy:**
```toml
[backends.my_backend]
service = "http"
[backends.my_backend.options]
base_url = "https://api.example.com/v1"
```

**New Style:**
```toml
# config.toml
[targets.my_api]
connection.host = "api.example.com"
connection.protocol = "https"
connection.base_path = "/v1"

# pipelines/main.toml
[backends.my_backend]
service = "http"
target_ref = "my_api"
```

### DICOM Backend (SCU)

**Legacy:**
```toml
[backends.pacs]
service = "dicom_scu"
[backends.pacs.options]
aet = "ORTHANC"
host = "localhost"
port = 4242
```

**New Style:**
```toml
# config.toml
[targets.orthanc]
connection.host = "localhost"
connection.port = 4242
connection.protocol = "dicom"

# pipelines/main.toml
[backends.pacs]
service = "dicom_scu"
target_ref = "orthanc"
[backends.pacs.options]
aet = "ORTHANC" # AET is still service-specific
```

## Precedence Rules

When both references and direct settings are present, the following precedence applies (highest wins):

1. **Service Options** (`options.*`): Highest priority. Preserves legacy behavior.
2. **Direct Connection Settings**: Settings defined directly on the endpoint/backend (e.g., `backends.my_backend.connection.host`).
3. **Referenced Settings**: Settings inherited from `peer_ref` or `target_ref`.

This allows you to inherit defaults from a target but override specific fields (like timeout) for a specific backend.
