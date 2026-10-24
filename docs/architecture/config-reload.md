---
title: Config Hot-Reload Strategy
description: How Harmony classifies configuration changes and applies them without a full restart.
status: active
updated: 2026-10-24
type: Architecture
related:
  - configuration/hot-reload.md
  - integrations/runbeam-cloud.md
---

# Config Hot-Reload Strategy

## Overview

This page describes how reload works inside Harmony. For what users see, the table of reload behaviour per config table, limitations and best practices, see [Hot Configuration Reload](../configuration/hot-reload.md).

Reload has three parts:

- `ConfigWatcher` in `src/config/watcher.rs` watches files and runs each reload.
- `compute_diff` in `src/config/reload.rs` compares the old and new `Config` and returns a `ConfigDiff`.
- `apply_reload` in `src/config/watcher.rs` publishes the new config, then stops, restarts and starts networks through `AdapterRegistry`.

## Reload Flow

```
┌──────────────────────────┐
│ notify watcher           │ main config file (non-recursive)
│                          │ pipelines dir (recursive, if it exists at startup)
└────────────┬─────────────┘
             │ modify event
             ▼
   sleep 200ms; skip if last successful reload < 1s ago
             │
             ▼
┌──────────────────────────┐
│ Config::load             │──── fails ───► log error, keep old config
└────────────┬─────────────┘
             │
             ▼
   update provider resolver
             │
             ▼
┌──────────────────────────┐
│ compute_diff(old, new)   │──── no changes, no failed networks ───► done
└────────────┬─────────────┘
             │ WARN if process_restart_required is not empty
             ▼
┌──────────────────────────┐
│ apply_reload             │
│ 1. globals::set_config   │
│ 2. stop removed networks │
│ 3. restart networks      │
│ 4. start added networks  │
└──────────────────────────┘
```

`src/lib.rs` creates the watcher in `run_with_reload`. It resolves the pipelines directory from `proxy.pipelines_path`, relative to the config file. It passes the directory to the watcher only if it exists at startup. The transforms and mesh directories are not watched.

### Event handling

The `notify` callback sends a message on a channel for each modify event. The loop takes one message, sleeps 200ms, then checks the time since the last successful reload. If that is under one second, it drops the message and takes the next one. Messages are not merged or queued for later. A failed reload does not reset the timer, so the next change after a failure always reloads.

### Config publication

The live config is an `ArcSwapOption<Config>` in `src/globals.rs`. Readers call `globals::get_config()` and get an `Arc<Config>` without a lock. The writer calls `globals::set_config()`. HTTP, HTTP/3 and DIMSE requests read the live config. For HTTP and HTTP/3, route lookup and the pipeline run use one snapshot, read when the request arrives. Each HTTP router rebuilds its route table the first time a request sees a new config.

## compute_diff

`ConfigDiff` has five lists. Each entry is a network name or a top-level table name, such as `"middleware"`. Entries are never field paths.

| Field | Contents |
|-------|----------|
| `zero_downtime_changes` | Changed tables that the swap applies |
| `adapter_restarts_required` | Kept networks to restart |
| `networks_to_add` | Networks only in the new config |
| `networks_to_remove` | Networks only in the old config |
| `process_restart_required` | Changed tables read wholly or partly at startup |

### Per-network checks

For each network in both configs, `compute_diff` adds the network to `adapter_restarts_required` if any of these differ:

1. `tcp_config` bind address or port. A missing `tcp_config` counts as the default.
2. `enable_wireguard` or `interface`.
3. Any other `NetworkConfig` field, such as `http3`.
4. The adapter set from `AdapterRegistry::required_protocols`. A network starts only the adapters its pipelines need: HTTP and HTTP/3 for HTTP endpoints, DIMSE for `dicom_scp` endpoints. Adding the first or removing the last pipeline of a protocol changes the set.
5. The DIMSE listener signature from `adapters::dimse::listener_signature`.

### Table classification

`classify_tables` destructures `Config` without `..`. Adding a field to `Config` fails to compile until the field is placed in a group. A table left out of every group would make a reload that changes only that table look like no change, and the watcher would drop it.

| Group | Tables | Reason |
|-------|--------|--------|
| Zero downtime | `middleware`, `policies`, `rules`, `pipelines`, `endpoints`, `backends`, `authentications` | Read when used, so the swap applies them |
| Zero downtime | `targets`, `peers`, `management` | Merged into backends, endpoints and pipelines at load, so their effect also shows in those tables |
| Restart every kept network | `mesh`, `ingress`, `egress`, `remote_ingress` | Each HTTP router indexes mesh ingress URLs when it starts |
| Process restart | `logging`, `storage`, `provider`, `proxy`, `runbeam`, `services`, `middleware_types`, `extensions` | Read wholly or partly at startup |
| Ignored | `network` | Compared per network |
| Ignored | `transforms`, `resolved_transforms_path`, `resolved_mesh_path`, `unresolved_backends`, `endpoint_sources` | Derived from other fields, or not configuration |

Process restart detail:

- `logging`: the log subscriber and log file are set up once. `RUST_LOG` is also read only at startup.
- `storage`: the storage backend is created once.
- `provider`: the watcher calls `update_providers` on the provider resolver on every reload. Cloud polling keeps the interval it started with.
- `proxy`: the watched pipelines directory is fixed at startup. Some fields are read live.
- `runbeam`: cloud polling starts at startup. Some fields are read live.
- `services` and `middleware_types`: these fill registries that can be set only once.
- `extensions`: plugins read these when they start.

The new values are still swapped in. Anything that reads the live config sees them. The watcher logs one WARN naming the tables.

## apply_reload

`apply_reload` takes a `NetworkControl` trait object. `AdapterRegistry` implements it. Tests use a recorder.

### Swap before restart

`apply_reload` calls `globals::set_config` before it touches any network. A restarted adapter, and anything that reads the global config such as the DIMSE peer check, then never runs under the old config.

It then works in this order:

1. Stop each network in `networks_to_remove`.
2. Restart each network in `adapter_restarts_required`, then each retried network.
3. Start each network in `networks_to_add`.

It attempts every network even after one fails. If any fail, it returns one error that names each failure. The new config stays published, because it passed validation before this point.

### Retry set

`ConfigWatcher` holds `failed_networks`, a `Mutex<HashSet<String>>`. A network that fails to restart or start goes into the set. On the next reload, `apply_reload` restarts every network in the set that is still configured and not already in the diff. It then refills the set with that reload's failures. A removed network drops out of the set.

The watcher runs `apply_reload` even when the diff is empty if the set is not empty. That is how saving an unchanged file retries a failed network.

### Network restarts

`AdapterRegistry::restart_network` calls `stop_network` then `start_network`. It restarts every adapter on the network, not one adapter. `stop_network` cancels each adapter's shutdown token, waits up to `ADAPTER_SHUTDOWN_TIMEOUT` (10 seconds), then aborts the task. It awaits the aborted task too, so the listener closes before the restart binds the port. Connection tasks an adapter spawned are not children of its task (AURA-2515).

### DIMSE listeners

The DIMSE listener reads endpoint and backend options only when it starts. `listener_signature` captures the `dicom_scp` endpoints and persistent-SCP backends a network serves, with their port, bind address, AE title, enabled operations, `use_dcmtk_store` and `persistent_store_scp`. Any change restarts the network.

This includes every option of a legacy `dicom_scu` backend with `persistent_store_scp` on a network that has a `dicom_scp` endpoint, even `host` or `aet`.

Within a network, the restart stops the old listener, including a DCMTK `storescp` process, before the new one binds the port. Networks restart in no fixed order. A port moving from one network to another may fail to bind until the other network has restarted. The retry set covers that case on the next reload.

### Mesh changes

A change to `mesh`, `ingress`, `egress` or `remote_ingress` adds every kept network to `adapter_restarts_required`, sorted by name. Networks restart one at a time. Until a network restarts, a request that matches one of its mesh ingress URLs gets 503. It never runs under mixed old and new mesh rules.

## Cloud Poller Handoff

The cloud poller in `src/management/cloud_poller.rs` does not reload. It writes files, and the file watcher reloads them. There is no other reload path.

For each pending change, oldest first, the poller:

1. Fetches the change and acknowledges it to Runbeam Cloud.
2. Downloads any transforms the change references into `proxy.transforms_path`. If that fails, it reports the change as failed and skips it.
3. Calls `write_cloud_config`, which writes the TOML by resource type:
   - `gateway`, and any unknown type: the main config file path.
   - `pipeline`: `<pipelines_path>/<name>.toml`.
   - `transform`: `<transforms_path>/<name>.json`.
   - `mesh`: `<mesh_path>/<name>.toml`.
4. Writes a backup copy to `./tmp/cloud_configs/<type>_<change_id>.<ext>`.
5. Reports the change as applied to Runbeam Cloud.

The poller reports "applied" once the file is written. That is before any reload runs. If the reload then fails validation, or a network fails to restart, the failure appears only in the gateway log. Runbeam Cloud is never told.

A pipeline change goes through a full reload, the same as any other save. There is no per-pipeline reload. Transform and mesh files land in unwatched directories, so they take effect on the next reload of a watched file.

The poll interval comes from the primary provider's `poll_interval_secs`, read once at startup in `src/lib.rs`. It defaults to 30 seconds. `[management]` has no poll interval.

## Test Coverage

- `src/config/reload.rs` unit tests: `compute_diff` for each table group, network port and add or remove changes, adapter set changes, and DIMSE listener changes.
- `src/config/watcher.rs` unit tests: the 200ms debounce, swap before restart, every network attempted after a failure, and the retry set.
- `tests/config_reload_integration.rs`: diff classification, invalid config rejection, adapter registry start, stop and restart, network add and remove, the config swap, file change detection, and the cloud poller writing a file that the watcher applies.
- `tests/e2e_run_with_reload.rs`: `run_with_reload` end to end, including adapter restart, invalid config rejection, and live policy and endpoint path changes on HTTP requests.

Run them with:

```bash
cargo test --test config_reload_integration
cargo test --test e2e_run_with_reload
```

## Future Enhancements

- Report reload results to Runbeam Cloud after the reload runs.
- Watch the transforms and mesh directories.
- Connection draining during network restart.
- A config change audit log.
- A reload dry-run mode.
