# Harmony Documentation

## Overview
Harmony is a proxy that enables secure communication between healthcare systems by handling FHIR and JMIX format exchanges, with support for HTTPS endpoints and WireGuard networking. It provides transformation rules between different formats and protocols commonly used in healthcare IT systems.

## Features

- FHIR endpoint support
- JMIX format handling
- WireGuard secure networking
- JWT authentication
- Audit logging
- Format transformation rules
- DICOM integration
- Configurable middleware middleware

## Installation

### Prerequisites

- Rust 1.87.0 or later
- WireGuard kernel module (if using WireGuard features)
- Linux/Unix-based operating system

### Building from Source

```aiignore
bash git clone <repository-url> cd harmony cargo build --release
```

The compiled binary will be located at `target/release/harmony`

## Configuration

### Basic Structure

The proxy is configured via a TOML file with the following main sections:

- `[proxy]` - Core proxy settings
- `[network]` - Network and HTTP binding configuration
- `[endpoints]` - External endpoint definitions
- `[internal_targets]` - Internal target configurations
- `[transform_rules]` - Data transformation rules
- `[middleware]` - Middleware configurations
- `[logging]` - Logging settings

### Example Configuration

```toml 
[proxy] 
id = "harmony-clinic-a" 
log_level = "info" 
store_dir = "/var/lib/jmix/studies"

[network] 
enable_wireguard = true 
interface = "wg0"

[network.http] 
bind_address = "0.0.0.0" # Listen on all interfaces bind_port = 8080

[endpoints.fhir_partner_a] 
type = "fhir" 
path_prefix = "/fhir/partner-a" 
middleware = ["jwt_auth", "audit_log"] 
group = "external_fhir"

[logging] 
log_to_file = true 
log_file_path = "/var/log/harmony.log"
```

### Required Directory Structure

```bash 
sudo mkdir -p /var/lib/harmony/studies 
sudo mkdir -p /var/log 
sudo chown -R {USER}:{USER} /var/lib/harmony
```
## Running the Service

### Command Line Usage

```bash
# Using default config location
harmony
# Specifying config file
harmony --config /path/to/config.toml
```

### Development Mode
```bash
# Run directly with cargo
cargo run -- --config examples/proxy-config.toml

```

### Production Deployment

1. Install the binary:

```bash
sudo cp target/release/harmony /usr/local/bin/
```

2. Create systemd service:

```ini
# /etc/systemd/system/harmony.service
[Unit] 
Description=HARMONY 
target 
After=network.target
[target] 
Type=simple 
User=jmix 
ExecStart=/usr/local/bin/harmony --config /etc/jmix/harmony-config.toml 
Restart=always
[Install] 
WantedBy=multi-user.target
```

3. Enable and start:

```bash 
sudo systemctl enable harmony 
sudo systemctl start harmony
```

## Monitoring

### Service Status

```bash 
sudo systemctl status harmony
```

### Log Access

```bash
# Systemd logs
journalctl -u harmony
# File logs
tail -f /var/log/harmony.log
```

## Security Setup

### File Permissions

```bash
# Config directory
sudo chmod 750 /etc/jmix sudo chown -R jmix:jmix /etc/jmix
# Log directory
sudo chmod 755 /var/log sudo chown jmix:jmix /var/log/harmony.log
# Data directory
sudo chmod 750 /var/lib/jmix sudo chown -R jmix:jmix /var/lib/jmix
```

### JWT Authentication

Ensure proper configuration of JWT authentication:

```toml
[middleware.jwt_auth] 
jwks_url = "https://your-auth-server/.well-known/jwks.json" 
audience = "harmony"
```

## Development

### Running Tests

```bash
# Run all tests
cargo test
# Run specific test
cargo test test_name
# Run with logging
RUST_LOG=debug cargo test
```

## Licence and Use

Harmony Proxy is licensed under the Apache License, Version 2.0.

**Important:** You may freely download, use, and modify Harmony Proxy for internal use and self-hosted deployments.

However, **reselling Harmony Proxy as a hosted service or embedding it in a commercial offering** requires a commercial licence from Aurabox Pty Ltd. Please contact us at support@aurabox.cloud for licensing enquiries.
