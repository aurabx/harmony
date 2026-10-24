---
title: Hot Configuration Reload
description: Which configuration changes reload without a restart, and how to use reload.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/configuration-file.md
  - architecture/config-reload.md
---

# Hot Configuration Reload

Harmony applies most configuration changes while it runs. Save the config file and Harmony reloads it. This page is part of [Configuration](configuration-file.md). For how reload works inside Harmony, see [Config Hot-Reload Strategy](../architecture/config-reload.md).

**Status**: Available since v0.4.0

## How It Works

Harmony starts a file watcher when you run it with a config file. The watcher reloads on file changes only. There is no reload endpoint in the Management API and no reload signal.

The watcher watches two places:

- The main config file.
- The pipelines directory (`proxy.pipelines_path`), and everything under it. Harmony watches it only if it exists at startup.

When a watched file changes, Harmony:

1. Waits 200ms for the file to settle.
2. Loads and validates the whole config. If that fails, it logs an error and keeps the old config.
3. Compares the new config with the running one, table by table.
4. Swaps in the new config. New requests use it at once.
5. Restarts each network whose listeners must change.
6. Logs what changed.

## What Each Table Needs

| Table | On reload |
|-------|-----------|
| `[middleware]`, `[policies]`, `[rules]` | Zero downtime |
| `[pipelines]`, `[endpoints]`, `[backends]` | Zero downtime, except DIMSE listener changes (see below) |
| `[authentications]`, `[targets]`, `[peers]`, `[management]` | Zero downtime |
| `[network.<name>]`, any field | Restarts that network |
| New or removed `[network.<name>]` | Starts or stops that network |
| `[mesh]`, `[ingress]`, `[egress]`, `[remote_ingress]` | Restarts every network |
| `[logging]`, `[storage]`, `[provider]`, `[proxy]`, `[runbeam]`, `[services]`, `[middleware_types]`, `[extensions]` | Process restart needed for full effect |

Some pipeline, endpoint and backend changes also restart a network:

- A network gains its first pipeline of a protocol, or loses its last one. A network starts only the adapters its pipelines need.
- A change to the `dicom_scp` endpoints or persistent-SCP backends a network serves, or to their options.

A network restart stops every adapter on that network and starts them again. Other networks keep running.

### Process restart tables

A reload still swaps in the new values for these tables. Code that reads the live config sees them. Code that read them at startup does not. Harmony logs a WARN that names the tables. Restart Harmony to apply them fully.

What does not change until a restart:

- `[logging]`: the log level and log file. `RUST_LOG` is also read only at startup.
- `[storage]`: the storage backend.
- `[provider]`: the cloud polling interval. The provider list itself updates on reload.
- `[proxy]`: the watched pipelines directory, and other `[proxy]` values read at startup.
- `[runbeam]`: whether cloud polling runs.
- `[services]` and `[middleware_types]`: the service and middleware type registries.
- `[extensions]`: settings that plugins read when they start.

## Usage

Run Harmony with a config file:

```bash
cargo run -- --config config/config.toml
# or
./harmony --config config/config.toml
```

Edit the file and save it. Harmony reloads it.

To check a config before you save it over the live one, run:

```bash
./harmony --config path/to/new-config.toml --validate-config
```

## Examples

**Example 1: add middleware (zero downtime)**

```toml
[middleware.my_transform]
type = "transform"

[middleware.my_transform.options]
spec_path = "my_spec.json"
```

Harmony swaps in the new config. The next request that runs this middleware uses it. Log output:

```
✓ Config reloaded successfully
  Zero-downtime changes: ["middleware"]
```

**Example 2: change a port (network restart)**

```toml
[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8081  # was 8080
```

Harmony restarts every adapter on the `default` network, including DIMSE and HTTP/3. Requests to that network fail until the new listener is up. Log output:

```
✓ Config reloaded successfully
  Networks restarted: ["default"]
  Networks added: []
  Networks removed: []
```

**Example 3: add a network**

```toml
[network.secondary]
interface = "eth1"

[network.secondary.http]
bind_address = "0.0.0.0"
bind_port = 8082
```

Harmony starts adapters for `secondary`. The `default` network keeps running.

**Example 4: add HTTP/3 to a network**

```toml
[network.default.http3]
bind_address = "0.0.0.0"
bind_port = 443
cert_path = "/etc/harmony/certs/fullchain.pem"
key_path = "/etc/harmony/certs/privkey.pem"
```

This changes the `default` network, so Harmony restarts that whole network. After the restart, HTTP on TCP and HTTP/3 on UDP port 443 serve the same pipelines.

## Errors

### Invalid configuration

Harmony keeps the old config and keeps serving. It logs the reason:

```
❌ Config reload failed: Config validation failed: Invalid network 'default': interface is empty
```

### A network fails to restart

This happens after validation passed, for example when the new port is in use. Harmony behaves as follows:

- The new config is already active. Harmony does not roll back.
- Harmony still tries every other network.
- A network that failed may be left stopped.
- Harmony remembers the failed network and restarts it on the next reload, even if that reload changes nothing.

Log output:

```
❌ Config reload failed: New config is active, but some networks failed and will be retried on the next reload: restarting 'default': <reason>
```

To retry after you fix the cause, save the config file again. The next reload logs `Retrying networks that failed on an earlier reload: ["default"]`.

## Limitations

- **One second between reloads.** After a successful reload, Harmony drops file changes for one second. It does not queue them. If you save twice inside a second, save again to apply the second change.
- **Unwatched directories.** Changes under `proxy.transforms_path` and `proxy.mesh_path` do not trigger a reload. A pipelines directory created after startup is not watched either. Save the main config file to reload after such a change.
- **No log for unchanged saves.** A save that changes nothing logs nothing.
- **Network restarts interrupt traffic.** Harmony stops a network's adapters before it starts new ones. New requests to that network fail until the new adapters are up. Harmony waits up to 10 seconds for the old adapters to stop, then aborts them. Requests still running at that point fail.
- **No rollback.** A failed network restart leaves the new config active.
- **No audit trail.** Harmony does not record a history of config changes.

## Best Practices

1. Run `--validate-config` on a new config before you save it over the live one.
2. Prefer zero-downtime changes where you can.
3. Make network and mesh changes during quiet periods. Mesh changes restart every network.
4. Read the logs after each save. Check for the `✓` line or a WARN.
5. Restart the process after you change a process restart table.
6. Keep copies of earlier configs so you can roll back by saving the old file.

## Troubleshooting

**Changes are not detected**

- Check the startup log for `📡 Watching config file for changes`.
- Check whether the change is in an unwatched directory.
- Wait one second after the last reload, then save again.
- Harmony reacts only to file modify events. Some editors save by writing a new file and renaming it. That may stop the watch. Restart Harmony if saves stop being detected.

**Reload failed**

- Read the `❌ Config reload failed` line. It names the cause.
- Run `--validate-config` on the file.
- Check that referenced files, such as transform specs and certificates, exist.

**A network did not come back**

- Look for `restarting '<name>'` in the failure line.
- Check for a port already in use.
- Fix the cause and save the config file again.
