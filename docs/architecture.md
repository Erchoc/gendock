# Architecture / 架构约定

## Scope

macOS Apple Silicon first. Rust owns the CLI, storage, queue and resident HTTP service. Existing MLX implementations may run in private Python workers; the public API must not expose Python setup. Linux, Windows and GUI products are deferred.

## Storage contract

Always use XDG roots, including on macOS. Never silently switch the CLI to Application Support. A future GUI can have separate defaults and explicitly connect to a selected CLI service. No global environment mutations.

```text
$XDG_CONFIG_HOME/gendock/
  config.toml                   # models, remote connection, export directories, power settings
  launchd/gendockd.plist         # canonical macOS login registration
  server-tokens.json            # owner-only named bearer credentials
$XDG_DATA_HOME/gendock/
  database/gendock.sqlite3       # implemented
  models/<id>/<revision>/       # verified managed snapshots; partial stages beside them
  runtimes/image.json           # private component registration; installer pending
  assets/<prefix>/<asset-id>/   # generated PNG assets
  uploads/<owner-hash>/<uuid>/  # authenticated caller input bytes
$XDG_STATE_HOME/gendock/
  logs/image/                  # generation logs; retention pending
  logs/server.jsonl            # structured lifecycle/queue events
  logs/gendockd*.log            # daemon/launchd console output, separate from JSON events
  server.json                  # instance receipt, PID and local origin
  server.lock                  # one service per state root
$XDG_CACHE_HOME/gendock/
  downloads/                    # planned, incomplete downloads
  previews/                     # planned, rebuildable
  work/<job-id>/                # planned, disposable intermediates
```

The OS-required `~/Library/LaunchAgents/gendockd.plist` is a symlink to the XDG canonical plist. User login starts the service through launchd; this is not a root system daemon before login. Explicit environment roots preserve XDG behavior, and tokens never enter the plist. Registration serializes updates, checks ownership, rolls back failures and supports owned unregister.

When HOME is absent or not absolute, fail clearly. XDG relative overrides are ignored per spec. Read-only path discovery creates no files. Runtime sockets should use a validated `XDG_RUNTIME_DIR` or a private per-user temporary directory; do not assume that macOS sets XDG_RUNTIME_DIR.

Model references resolve symlinks and store the canonical path. Referenced files are never changed or deleted. Current registration accepts UTF-8 paths only and does not validate model compatibility. Reference path updates validate existence and preserve ownership. Managed model roots can be selected per download with `--models-dir`; model storage defaults and managed asset-root configuration are planned. Export directories are configurable independently through `output.*`; see [configuration.md](configuration.md). A missing external disk is an availability error, not permission to remove records.

Future asset references use stable IDs and storage locations rather than embedding absolute paths throughout job requests. Cloud storage and compute selection remain independent. Do not sync live SQLite files through cloud drives; use APIs and object storage.

## Database and queue

SQLite is used for structured local state (catalog/registrations, task history and asset indexes), not because a GUI is planned. Configuration stays in TOML and large files stay on disk. The CLI requires no database server or ORM; synchronous image commands do not require a daemon or background scheduler. Do not treat a daemon as a prerequisite for local synchronous commands; it is nevertheless required for the shared-device mode.

SQLite: WAL, synchronous FULL, foreign keys ON, five-second busy timeout. The DB belongs on a local filesystem. Back up using SQLite's backup facilities or a safely closed database, not by copying only a live main file. Never delete WAL/SHM files manually.

Before 1.0 all DDL lives in the single `migrations/001_initial.sql` snapshot; there is no incremental migration chain. Empty databases are initialized inside an immediate transaction, and incompatible development schemas are rejected with `database_schema_incompatible` without data deletion or reset. The current marker remains 3 so existing compatible development databases stay usable; it is unrelated to the application release number. Structural changes to this snapshot require an explicit compatibility-marker decision. After 1.0, released databases will use ordered transactional migrations. Built-in catalog data reconciliation remains independent and automatic. Search reads SQL, not the seed JSON. User definitions are isolated from built-in updates, and omitted built-ins become deprecated without file deletion. Lock initialization before checking user_version to avoid concurrent DDL races. Jobs persist JSON requests, states and timestamps. The atomic UPDATE RETURNING claim prevents two connections from claiming the same queued row. The CLI exposes listing/details and direct synchronous image execution; direct callers never claim queued tasks. The service uses a separate atomic, fenced claim for its queued jobs and serializes both paths with the same OS image lock. Synchronous jobs record CLI/worker PIDs; abandoned jobs are reconciled conservatively, and SIGINT/SIGTERM terminate/reap the owned process group. See docs/image-generation.md.

Service job JSON carries submitter identity, resolved submission/model revision, instance ownership, heartbeat/lease and cancellation. Existing five tables and schema marker 3 remain compatible. Claim, admission limits and transitions use short transactions/statements; never hold a write transaction during hashing/inference. Heartbeat/cancellation checks also run during model verification. Recovery holds the GPU lock and treats live/reused worker PIDs or fresh live ownership leases conservatively. Interrupted tasks are not automatically replayed; a persisted job does not provide exactly-once execution or idempotent submission.

## Shared-device service requirement

A resident service, durable queue and completion notifications are required product capabilities for 20+ remote clients sharing one Mac. The implemented service authenticates named bearer credentials from an owner-only file, authorizes jobs/results by identity and resolves caller-scoped upload IDs. Start with one executing GPU job and FIFO. Pending limits are transactional; uploads/retained assets still need byte quotas and cleanup. CLI/API share the verified image execution core and managed asset archive. Clients verify downloads and export locally; server metadata does not record foreign client paths. SSE sends immediate/current status snapshots, changes, heartbeats and terminal closure; reconnect sends current persisted state, not event history. CLI waiting falls back to polling. Normal stop preserves queued work; crash recovery never blindly kills an unknown/reused PID. Keep future schema growth justified; webhook delivery needs persisted retry state only when implemented. HTTP residency is distinct from model residency. See [server.md](server.md) for API and limits, and [testing.md](testing.md) for acceptance.

## Runtime isolation

Do not discover Python from PATH or use user virtualenvs. Future adapters select a pinned, versioned private interpreter and locked dependency set. Verify downloaded artifacts, stage installations, activate atomically, preserve rollback and constrain subprocess environments. Do not pass user prompts through shell interpolation. Use structured IPC; separate logs from protocol messages. Native wheels and supported macOS versions require real compatibility checks.

## API boundaries

`config`: atomic non-clobbering initialization and strict TOML defaults; execution reads separate model preferences, never auto-downloads missing models. `image/input`: bounded input decoding and immutable editing snapshots.
`image`: configured model selection, preflight, synchronous generation/edit lifecycle, asset/export handling. `runtime/`: private component registration, isolated subprocess adapter, cancellation/log capture. CLI callers never select an engine or Python. A preinstalled private component can be registered for development; the automatic pinned installer is pending.

`client`: selected/discovered connection, SSE/polling wait, uploads, verified downloads and client-local exports. `server`: authenticated HTTP routing, lifecycle and one queue consumer. `store/queue`: transactional admission, fenced claiming, heartbeat, owner visibility and cancellation, with no terminal presentation.

`models/`: typed definitions and manifest validation, device estimates, HTTP transport and managed downloads. `store/catalog.rs`: SQL search, user maintenance, legacy backfill and monotonic built-in reconciliation. Direct file URLs may be R2/CDN; HF is one source. See `docs/models.md`.
`paths`: pure resolution plus environment discovery.
`store`: durable models and jobs, no inference or terminal output.
`main`: clap parsing and presentation.

`cli/`: typed clap commands and JSON output. `app`: application dispatch, implemented operations and explicit not-implemented failures. `main`: parse errors/help/version and process exit handling. Operation results/errors emit JSON by default; successful foreground `server start` streams leveled console logs instead, and startup failures remain JSON. Background/registration control acknowledgements remain JSON. Help-only invocations emit text and exit 0. Internal path discovery remains a library function, not a user-facing system command.

Keep a single crate with a library and binary until concrete boundaries justify a workspace. Do not scaffold dozens of empty crates. Future CLI and HTTP clients must share service logic. Unsupported backends and operations must fail explicitly. Recommended models must resolve to pinned revision/backend/quantization details recorded per task.
