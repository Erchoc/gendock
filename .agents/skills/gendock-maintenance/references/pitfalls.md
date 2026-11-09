# Known pitfalls

## Release runners and lockfiles

GitHub rejected `macos-14` before any steps ran with `macos-14-arm64-us-default is not supported`; macos-15 started successfully. A clean runner has no Cargo registry cache: `cargo update --offline --workspace` failed even when the app version was unchanged. Release planning now edits only the single gendock package version in Cargo.lock and leaves dependency identities intact; subsequent `--locked` validation verifies consistency. Planner regression tests check threshold 3/4 and dependency-version preservation.

## macOS CLI paths

Do not use platform-default directory helpers that silently select Library/Application Support. The CLI deliberately follows XDG even on macOS. GUI defaults are a separate future decision.

## External model ownership

Canonicalizing a path does not transfer ownership. Registration removal must only delete the row. A successful registration proves existence, not backend compatibility. Unplugged disks must not trigger automatic removal.

## SQLite WAL

Committed data may still live in the WAL. Copying only the main file while running can lose data. Use SQLite backup APIs. WAL allows concurrent readers and a writer, not multiple simultaneous writers. Claiming jobs must be atomic; the service now stores leases/heartbeats and authorizes ownership inside existing job JSON. Distinct connections must test admission limits and fenced claims, not just a single in-memory connection.

## Python isolation

A working web page does not prove an inference engine is installed. The reference Phosphene project can serve HTTP 200 using Python's standard library while generation dependencies are absent. Report capability readiness separately from service liveness. Never suggest the user's Python is broken merely because a private backend is missing.

## Rust toolchain selection

During bootstrap, mise selected Rust 1.95 via `RUSTUP_TOOLCHAIN`; its rustup installation was incomplete (missing manifest). Explicit `+stable` only bypassed the problem. The actual repair was `rustup toolchain install 1.95.0 --profile minimal --component rustfmt --component clippy`, which recovered the partial installation while preserving mise's version selection. Ordinary cargo formatting, clippy, tests and installation then passed on 1.95.0. Inspect mise and rustup together before changing toolchain defaults. Long downloads should run with progress checks rather than be repeatedly interrupted.

## Scope honesty

The initial project has storage and CLI discovery, not generation. An empty queue and an arbitrary model backend string do not mean an engine is usable. The user explicitly requested planned commands to be present as JSON `not_implemented` failures; this supersedes the earlier preference to hide placeholders. Stubs exit 3 and have no side effects. Operation results/errors and version return JSON by default; bare invocation, bare command groups and help print human-readable text with exit 0. No `--json` flag. Never return fake successful generation responses.

## Hugging Face manifests and mirrors

The guessed repository `mlx-community/FLUX.1-schnell-4bit` returned an HF authentication-style error even though the root problem was a nonexistent repository. Query the public API and verify exact IDs before promising availability. Initial snapshots use actual mflux-format repositories and immutable commits. Large LFS files have SHA-256 identities; small ordinary files use Git blob SHA-1 with the `blob <size>\0` prefix. Raw-file SHA-1 is wrong. Do not trust mirror-provided hashes to establish identity; compare with the immutable identities stored in the local catalog (seeded/reviewed for built-ins, declared by the owner for user entries).

An HTTP server may ignore Range and return 200. Truncate and restart in that case; append only after validating a 206 Content-Range against the expected offset and total. Do not register or activate a corrupt snapshot. Lock the model directory across processes, keep valid partial files, and verify completed snapshots on reuse. Distinguish mirror selection from network proxies and never forward official HF tokens to third-party endpoints.

macOS temporary paths may traverse `/var` symlinks. Canonicalize the user-selected storage root first, then reject symlinks inside managed content so checks do not incorrectly reject normal system directory aliases.

## SQLite model catalog and release reconciliation

Users expect `gendock model search flux`; requiring only `--query` was an unnecessary friction. The positional form is primary and the former flag remains compatible. Search must query SQLite and return all types with `type`, not filter by type. The earlier read-only/no-database discovery rule was superseded by the user's explicit database-backed catalog requirement: search/info may seed/reconcile local storage, but never fetch remote metadata or weights.

Separate catalog metadata from local file ownership. Bump `BUILTIN_CATALOG_REVISION` on release-seed edits; transactions preserve user rows, deprecate omitted built-ins, and never delete files. A lower catalog revision cannot undo a newer recommendation. User JSON cannot gain builtin ownership or replace/delete built-in rows. Acquire an initialization transaction before checking user_version; otherwise concurrent first opens can both attempt initial DDL.

Direct R2/CDN URLs must not be rewritten into HF paths, and using the default HF endpoint must not cause HF_TOKEN to be attached to those requests. Tests run the real CLI against a local direct URL with a fake token and verify no Authorization header is sent.

## Standalone CLI and model inventory

An empty Git checkout or empty default HF cache does not prove the machine has no MLX tools. In local validation, mflux 0.18.0 / MLX 0.31.1 already existed in Pinokio's private Python environment, with commands outside PATH; model weights were under Pinokio drive symlinks. LTX/Gemma weights are not standalone image models merely because they end in .safetensors. Check metadata and task capability, not file extension alone.

Strict proxy bypass means removing both uppercase/lowercase HTTP/HTTPS/ALL proxy variables and setting NO_PROXY/no_proxy to `*`. On the tested network official HF reset direct connections, while HF Mirror worked directly; this is an observed network condition, not a universal availability guarantee.

A verified FLUX.2 Klein 4B q4 snapshot ran offline through mflux CLI for both generation and editing. See `docs/mflux-cli-validation.md`. The specific weight metadata version was newer than the installed runtime but this smoke check passed. Do not infer that every model version is compatible or that a complete managed-runtime installer exists.

## First integrated image adapter

`gendock image generate` now invokes one registered private component synchronously, with model/runtime/output validation and SQLite job/asset records. The real wrapper was tested with a Chinese prompt on M4 Pro; mocks only verify orchestration, not MLX correctness. Keep component choice internal and normal commands model-oriented. Existing local references infer component metadata from known catalog IDs; the historical backend option remains hidden for compatibility.

A reused private environment plus entrypoint fingerprint is a development bridge: it does not pin every dependency or satisfy full standalone distribution yet. Do not discover system Python, infer compatibility solely from an executable name, or expose setup flags to normal callers.

Capture worker stdout/stderr in a log file, not JSON stdout. Invoke via an argument array; prompts may contain quotes, newlines and shell syntax. A worker exit 0 still requires PNG decoding/dimension validation. Export atomically without clobbering; if export fails after successful generation, retain the succeeded job and original asset. Persist effective parameters separately because mflux metadata sidecars were null in the tested runtime.

Cancellation must kill/reap the owned child before releasing the image lock. SIGKILL can leave a worker alive; later commands must conservatively refuse a live/reused PID rather than start a second job or blindly kill it. Synchronous execution remains separate from queue claiming. The service reuses its existing queued job ID in the same execution core, and heartbeats/cancellation cover hashing as well as inference.

Default exports now belong in the invocation directory, not only the managed asset store. Capture local month/day/hour/minute at invocation; atomically choose image_MMDD_HHMM.png then _2/_3 suffixes. Never select by exists-check alone, as another writer can win the name. Tests must set their own temporary current directory so default exports never pollute the repository.

Local HTTP test listeners are nonblocking for bounded accept loops. Explicitly set each accepted stream back to blocking before reading headers; macOS can otherwise produce sporadic EWOULDBLOCK even with a read timeout.

## Configured defaults and editing

Defaults are per-operation preferences in XDG config.toml, not catalog recommendation flags or hardcoded runtime decisions. Preserve existing files, reject unknown/empty/malformed settings, and use atomic no-clobber initialization. Explicit --model overrides the selected default only for that request. Generation/editing never silently download or fall back when a configured model is unavailable; return model_not_installed plus a structured download argv.

Edit inputs are decoded with limits and copied byte-for-byte into the task's private asset directory before launch. Record source/snapshot/hash/dimensions and invoke the edit entrypoint with --image-paths pointing to the snapshot. Merely calling the generation entrypoint with an input path does not implement instruction editing. Both operations share locking, cancellation and result validation but retain distinct job kinds.

A four-second fixed startup assumption became flaky as CLI fixture tests ran in parallel. Poll process exit separately from the readiness marker, bound the startup allowance, and kill/reap on timeout with diagnostics rather than leaking a child or hiding an early CLI failure.

## Global configuration administration

`config list/get/set/unset/path` uses effective defaults, strict typed validation and redacted API key results. Hold a stable sidecar lock across read/modify/atomic replacement; locking the replaced file itself can lose updates. CLI updates reserialize TOML and preserve unrelated settings, but not comments. Validate before replacing and keep managed config files owner-only. `config path` must remain side-effect free. Tests cover concurrent independent updates and rejected updates preserving exact prior bytes.

Output preferences affect exported copies, not managed asset/history roots. Explicit relative `--out` always resolves against invocation cwd, even with an output directory configured. Editing uses `output.image_edit` then `output.image`; service-side execution must not export to a client's host path. Task-scoped caffeinate assertions belong to the inference host and must reap their owned helper on drop.

## Pre-1.0 schema policy and CLI scope

The user explicitly chose one complete SQL snapshot before 1.0. This supersedes the earlier incremental-migration convention: edit migrations/001_initial.sql in place, do not add upgrade scripts, initialize empty DBs transactionally and never reset incompatible DBs automatically. Retaining marker 3 during consolidation preserved the existing local DB without touching models/history/assets. Built-in model data revisions still reconcile independently; they are not structural migrations. Formal upgrades for released user databases begin after 1.0.

SQLite is an implementation choice for structured model/task/asset state, not a GUI prerequisite. A stateless wrapper could use only TOML/JSON/files, but replacing this project's existing shared state with mutable JSON would require concurrency and atomicity machinery. Keep the embedded DB thin, with no database service or ORM, and do not make a daemon/async scheduler a prerequisite for synchronous CLI features.

The user clarified that resident service, queue scheduling and completion notifications are still required for 20+ remote colleagues sharing a Mac. The concern was unnecessary SQL/table growth, not removing shared-device capabilities. This supersedes the temporary roadmap classification of the daemon/queue as optional. Preserve local synchronous use while implementing shared-device mode with serialized GPU work and authenticated ownership; avoid a new users/notifications subsystem unless concrete needs justify it. HTTP service residency and persistent model residency are different features.

## Shared-service streaming and recovery

`tiny_http::Request::into_writer` exposes a raw writer; putting `Connection: close` in hand-written headers does not ensure the underlying shared connection closes. The first SSE test received events but timed out waiting for the body to end. Use valid chunked framing, flush each event/heartbeat and send the terminating zero-length chunk at terminal state. Test that a running event arrives while execution is still active, then cancel it and verify the cancellation event and clean EOF. A terminal-only assertion misses buffered notifications.

The service and direct image callers must share the same OS GPU lock. Recovery needs to skip the currently executing queued job, preserve a fresh live claim-to-execution lease, and refuse to overlap a surviving/reused worker PID. An expired heartbeat never authorizes blindly killing a process. A busy-GPU handoff racing with cancellation must become cancelled, not strand a running task or silently requeue cancelled work. Tests cover independent SQLite connections, normal stop/restart and SIGKILL with a surviving component.

Keep component processes in an owned process group on Unix. On cancellation/error, kill the group only while the owned child remains unreaped, then reap the child. Do not send a group signal after reaping a completed child, when its PID could have been reused.

## HTTP rejection logging and address advertisement

Authentication used to return 401 before the main handler recorded anything, so rejected browser/API visits were invisible in the foreground console. Keep a request-lifetime access context around every response path, including auth failures, bounded-capacity rejections, files and SSE. Correlate X-Request-ID with status/duration/peer; classify 4xx as WARN and server-side 5xx as ERROR. Do not record bearer values, query strings or bodies. Preserve actual unknown paths for diagnosis (bounded to 2048 characters, JSON-escaped); replacing every unknown route with <unmatched> hides useful context. Test both console and JSONL output with injected credential-store failure and restore the fixture credentials before shutdown.

Enumerating a LAN IP does not mean a loopback-only socket accepts it. Startup/status must show localhost plus detected interface addresses with explicit binding availability, and hint --host when loopback-only. A bare --host opts into 0.0.0.0 LAN listening, while --host IP retains explicit binding. Discover interfaces locally without external probes; skip loopback/link-local addresses and bracket IPv6 URLs. Keep the default listener unchanged unless the user explicitly requests broader binding.

## Foreground startup and login registration

`server start` now defaults to foreground leveled console logs; startup failures still return one JSON document. Background callers/tests must explicitly use --daemon. Successful foreground shutdown must not append a success envelope. Keep structured server.jsonl distinct from human daemon/launchd console logs, and preserve the child's actual error envelope when daemon startup fails. A local bind conflict with an existing mlx_ltx_panel process exposed the previous generic server_start_failed wrapper.

macOS launchd registration stores its canonical plist in XDG config and creates only the required LaunchAgents symlink. Label gendockd is user-wide, so serialize mutations, reject foreign targets and test rollback. Escape paths via a plist serializer, pass explicit HOME/XDG roots and omit credentials. KeepAlive SuccessfulExit=false restarts failures while allowing a clean server stop to remain stopped until the next login or explicit start. The real launchd test uses a temporary HOME, refuses an existing job, verifies bootstrap/readiness/plutil/unregister and leaves no installed global job. It does not substitute for reboot/login acceptance.

## Real-model E2E and hash cost

Debug builds can exceed tool-call timeouts while hashing multi-GiB models, even before inference starts. The opt-in real E2E script uses `--release`. Verify only the manifest's selected checksum algorithm, not both SHA-256 and Git SHA-1 for every large file. Keep full identity checks; cooperative callbacks refresh the service lease and honor cancellation during hashing. A unit check interrupts verification and confirms original weight bytes are unchanged.

Real acceptance uses a separate temporary host/client XDG tree, read-only lookup of the existing model registration and a copied private-runtime manifest. Routine tests skip it. Retained `report.json` includes paths and decoded-pixel hashes without access tokens. Changed PNG bytes alone could just be metadata, so compare decoded pixels and preserve the original source hash; semantic image quality still needs visual review.
