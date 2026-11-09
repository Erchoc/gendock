# Shared-device service

The service supports the current FLUX.2 Klein image generation/editing recipe. Clients submit work to one durable FIFO queue; only one GPU job executes at a time. Local synchronous commands still work without a service. With a discovered/selected service, image commands submit to it and wait/export on the client unless `--async` is set.

## Device administrator

Prepare the local model and registered private components first; see [image-generation.md](image-generation.md). Starting a service never downloads components or models.

```sh
# Foreground: timestamped, leveled logs; color on a terminal
 gendock server start

# Background: returns a JSON acknowledgement
 gendock server start --daemon
 gendock server status
 gendock server logs

# Issue a separate token for each caller; JSON contains the token
 gendock server token --name alice

# Stop before changing the listener
 gendock server stop
 gendock server start --daemon --host --port 9966
```

Names contain 1–64 ASCII letters, digits, `_` or `-`. Reissuing a name returns its existing token. `admin` controls shutdown and sees all jobs; ordinary identities see/cancel/fetch only their own jobs. Tokens live in owner-only `$XDG_CONFIG_HOME/gendock/server-tokens.json`. Every HTTP route requires a bearer token, including status and browser visits. Missing/invalid credentials return 401 with a `WWW-Authenticate: Bearer` challenge. The local CLI can use the local administrator token automatically when discovering this machine's service; explicit URLs and remote clients use `GENDOCK_TOKEN` or `remote.api_key`. A browser URL visit does not attach that header automatically. Token rotation/revocation commands and organizational login are not implemented.

`server start` stays in the foreground and prints one timestamped `INFO`/`WARN`/`ERROR` log per line, with service name `gendockd`. Terminal logs use severity colors; redirected output is plain text. `NO_COLOR` disables colors and `CLICOLOR_FORCE=1` forces them. Successful startup/shutdown do not append a JSON result. Startup failures return the standard JSON envelope before any successful-start logs. Do not pipe a running foreground server into `jq`.

`--daemon` detaches and returns a JSON acknowledgement after readiness. `--register` installs and starts the macOS user launchd service; these flags are mutually exclusive. `--foreground` remains a hidden compatibility spelling for the default. `--port 0` chooses a free port. `server.host`, `server.port` and `server.max_pending_per_user` remain configurable. The default port is `9966`; an explicit `--port` overrides configuration for that start.

```sh
# macOS: start gendockd now and after subsequent user logins
 gendock server start --register --port 9966
 gendock server status | jq

# Stop this run; registration remains for the next login
 gendock server stop

# Stop/unload the registered service and remove its startup registration
 gendock server unregister
```

Registration is user-level and needs no sudo: it starts after login, not before any user logs in. launchd keeps a crashed/nonzero-exit service running, with a restart throttle; a clean `server stop` does not immediately respawn it. The plist is stored at `$XDG_CONFIG_HOME/gendock/launchd/gendockd.plist`; the OS-required `~/Library/LaunchAgents/gendockd.plist` is a symlink. Model/config/data/state/cache paths still follow XDG, and the launch job receives explicit HOME/XDG roots without access tokens. Registration preserves unrelated files, rejects foreign registrations and rolls back newly installed files when startup fails. `--register` requires stopping an already running service first; unregister before changing an already loaded registration's command/settings.

Startup logs, daemon acknowledgements and status include `addresses.local_url` and detected `addresses.lan_addresses` with interface names. Default loopback binding still lists detected LAN URLs, marked `accessible: false`, with a hint to use `--host`. The bare `--host` flag enables LAN listening (`0.0.0.0` internally); `--host IP` retains explicit address selection. Omitting the flag uses the configured listen address, normally loopback. Wildcard listening enables LAN interface URLs; binding one concrete LAN address enables only that address. `accessible` describes the listener's scope; routing and firewall policy still apply. Discovery enumerates local interfaces without sending external traffic. Restart an existing service to use changed listener settings or the updated binary.

Every handled HTTP request, including authentication failures and capacity rejections, emits an `http_request` console/structured event with request ID, method, actual request path (including unknown routes), peer address, authenticated identity when available, status code, elapsed milliseconds, error code and connection failure flag. 2xx/3xx log at INFO, 4xx at WARN, and 5xx at ERROR; a failed connection write also logs WARN. JSON/file responses and SSE carry `X-Request-ID` for correlation. Stream duration includes its lifetime. Authorization headers, query strings and request bodies are excluded; unknown routes preserve the supplied path, capped at 2048 characters with a truncation marker. JSON escaping preserves one-line log framing.

Structured event logs remain at `$XDG_STATE_HOME/gendock/logs/server.jsonl`; `logs --follow` emits JSON snapshots as NDJSON. Daemon console output uses a per-start `gendockd-UUID.log`; launchd console output uses `logs/gendockd.log`. Log rotation is pending.

A bind failure reports `server_bind_failed`, host, port and OS reason in foreground and daemon modes. For a busy configured port, choose another port or use `--port 0`; the CLI does not stop another application's process.

The listener supplies HTTP. Use an HTTPS reverse proxy or a trusted private network for remote access; TLS termination is external. IPv4 and concrete IPv6 addresses are supported. Wildcard IPv6 listening/discovery has not been separately validated.

## Remote caller

Install the Rust CLI on the client. Model weights and private inference components belong on the shared device, not the client.

```sh
export GENDOCK_URL=http://MAC_IP:9966
export GENDOCK_TOKEN='TOKEN_ISSUED_FOR_YOU'

# Returns a durable job_id immediately
 gendock image generate --prompt "A blue ceramic cup" --async
 gendock job wait --id 1 --timeout 600
 gendock job fetch --id 1 --out ./results

# Input bytes upload automatically; the original file is preserved
 gendock image edit --input ./results/job_1.png --prompt "Make the cup green" --async
 gendock job wait --id 2 --timeout 600
 gendock job fetch --id 2 --out ./edits

 gendock job list
 gendock job info --id 2
 gendock job cancel --id 3
```

Use the actual returned IDs. `--url ORIGIN` overrides `GENDOCK_URL`; global `remote.endpoint`/`remote.api_key` are also supported. A selected remote connection failure does not fall back to local inference. Without `--async`, image commands submit, wait, verify the download and export into the client's configured output directory or invocation directory. Explicit `--out` is a client path. `--async --out` is rejected: use `job fetch --out DIRECTORY` after completion. Fetch uses `job_ID.png` and atomic `_2`, `_3` suffixes. Export failures preserve succeeded server assets and include the job ID for recovery.

Model and configuration administration operate on the machine where the CLI runs; this API does not offer remote model installation. Server start/token/log inspection are local administration operations. Remote `server status/stop` use the selected connection; shutdown requires an administrator token.

## API and ownership

All routes require `Authorization: Bearer TOKEN`. For example, after exporting the caller's token:

```sh
curl -H "Authorization: Bearer $GENDOCK_TOKEN" "$GENDOCK_URL/v1/status"
```

JSON replies use the normal version-1 envelope. Remote job replies omit host paths, worker identities and private logs. Missing and inaccessible jobs both return `job_not_found`.

| Method | Route | Purpose |
| --- | --- | --- |
| GET | `/v1/status` | Service instance and execution policy |
| POST | `/v1/uploads` | PNG/JPEG/WebP bytes, up to 64 MiB; returns `upload_id` |
| POST | `/v1/jobs` | JSON task; returns `job_id` |
| GET | `/v1/jobs` | Caller-visible latest 100 jobs; administrator sees all owners |
| GET | `/v1/jobs/ID` | Current task state/result/error |
| POST | `/v1/jobs/ID/cancel` | Persist cancellation request |
| GET | `/v1/jobs/ID/events` | SSE current snapshot, changes and heartbeat; closes at terminal state |
| GET | `/v1/jobs/ID/file` | Verified successful PNG asset |
| POST | `/v1/shutdown` | Administrator-only graceful shutdown |

Submission fields: `kind`, `prompt`, optional `model`, `seed`, `aspect`, `width`, `height`, `steps`, `input_upload`. `kind` is `image.generate` without an upload or `image.edit` with an owned upload ID. Unknown fields, including filesystem paths and output paths, are rejected. Upload IDs are scoped to the caller even for administrative job inspection. Defaults and model revision are resolved when submitting; changed revisions fail explicitly instead of silently switching a queued task. Quick preflight checks file presence/size; execution verifies full file hashes and the registered component before launch.

```json
{"kind":"image.generate","prompt":"A blue cup","width":512,"height":512,"seed":42}
```

Uploads are stored under caller-scoped managed directories, then snapshot byte-for-byte into task assets. Uploaded files, job assets and logs currently require administrator-managed retention; quotas for retained bytes and cleanup commands are pending. Pending jobs are limited per caller (20 by default, including running work). The HTTP listener caps active handlers at 64; this is not a public-internet hardening or sustained-load guarantee.

## Cancellation, recovery and notification

Queued cancellation is terminal immediately. Running cancellation is persisted and kills/reaps the owned execution process group. Graceful shutdown interrupts active work, preserves pending tasks, and resumes pending tasks after restart. Interrupted/failed tasks are not automatically replayed. Unhandled client disconnection does not cancel an accepted task.

Worker ownership, heartbeat and a ten-second lease are stored inside existing job JSON; the schema remains the same five tables and marker 3. File verification refreshes the lease and checks cancellation too. Recovery runs while holding the OS image lock: a surviving/reused worker PID blocks new GPU work until safe to reconcile. A fresh live ownership lease also protects the claim-to-execution handoff. Heartbeat expiry alone never authorizes killing a process or overlapping inference. After a crash, a dead worker's task becomes interrupted; a live orphan can finish its component but its abandoned task is still reconciled as interrupted. Inspect its retained files rather than assuming exactly-once delivery or success.

SSE sends a snapshot immediately, then status changes and one-second heartbeats. Reconnection sends the persisted current state, including an already-terminal result. `Last-Event-ID` does not request historical replay; there is no event-history table. `job wait` uses SSE and falls back to polling after disconnection. A timeout preserves the job ID; wait again instead of submitting a duplicate. Failed/cancelled/interrupted waits exit 1 with `job_failed`; info/cancel successfully querying such a task still return a successful query response.

Durable webhook delivery, submission idempotency keys, richer progress, fair-share scheduling, retained-event history and persistent model residency remain future work. Current residency keeps the HTTP service alive; inference components still load models per task.

See [testing.md](testing.md) for reproducible CLI and real-model acceptance scripts.
