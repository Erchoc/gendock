# Image generation and editing

```sh
gendock image generate --prompt "A watercolor fox" --out fox.png
gendock image edit --input fox.png --prompt "Make the cup green; preserve everything else"
gendock image generate --model flux2-klein-4b-q4 --prompt "A cabin by the sea" --seed 42 --width 512 --height 512 --steps 4 --out cabin.png
gendock job info --id 1
```

No backend or Python arguments are required. The first integrated recipe supports both operations with the same FLUX.2 Klein 4B q4 weights. Other catalog entries still return `model_execution_unsupported`; changing a default is not a way to add a missing execution recipe. Retired models cannot execute; deprecated models require explicit selection. Both commands execute directly when no service is selected. A discovered or configured service queues the task, while the CLI waits and exports on the client; `--async` returns immediately. See [server.md](server.md).

## Default model configuration

Image execution creates `$XDG_CONFIG_HOME/gendock/config.toml` if absent (normally `~/.config/gendock/config.toml`):

```toml
[defaults]
image_generate = "flux2-klein-4b-q4"
image_edit = "flux2-klein-4b-q4"
```

Each command reads its own default at invocation. Manage settings with `gendock config list/get/set/unset/path`; see [configuration.md](configuration.md) for export directories, remote preferences and task-scoped macOS sleep prevention. `--model ID` overrides only that invocation and does not rewrite configuration. Existing configuration is never replaced by initialization or upgrades. A missing section/key uses the initial default; empty IDs, malformed TOML and unknown keys return `invalid_config` with the file path. Initialization is atomic under concurrent calls.

Execution **never downloads models or components automatically**. A catalog model without a local registration returns `model_not_installed`, its ID/config path and an explicit `gendock model download --model ID` recovery action. Missing/corrupt registered files return `model_files_incomplete`; a missing component returns `runtime_not_ready`. No worker/job is started for these preflight failures. Unknown IDs return `model_not_found`; unsupported recipes remain explicit errors rather than silently switching models.

## Inputs and parameters

Defaults for both operations: 512×512 output, four steps, random recorded 32-bit seed. Editing does not automatically inherit source dimensions; use `--width`/`--height` or `--aspect` to select output size. Dimensions must be 256–2048 and divisible by 16; steps 1–50. Supported aspects: 1:1, 3:2, 2:3, 16:9, 9:16, 4:3, 3:4. Only PNG output is supported. `--async` requires a running service, returns a durable job ID and pairs with `job wait/cancel/fetch`. It cannot be combined with `--out`; fetch after completion instead.

Editing accepts PNG/JPEG/WebP by detected content, validates decoding, and limits source files to 64 MiB, 8192 pixels per dimension and 256 MiB decoder allocation. The source is read once and archived byte-for-byte as `input.<extension>` in the new asset directory before execution. The component receives this snapshot, not the mutable original path. Request/result metadata retain source path, snapshot path, SHA-256 and dimensions. Original files are never modified or overwritten.

## Execution and results

- Resolve configuration, catalog metadata and local registration without network access.
- Locate the registered private component; never look up system Python.
- Serialize generation/edit GPU work with one OS lock.
- Check every required model file's size/hash, including tokenizer/config components.
- Persist operation (`image.generate` or `image.edit`), resolved parameters, model revision, component fingerprint, CLI/worker PIDs and input metadata.
- Run offline using an argument array; remove Python environment overrides and user-site packages. Capture all worker output in an XDG state log, separate from JSON stdout.
- Decode/check the PNG and requested dimensions, hash it, then atomically record a successful job and asset.
- Export without clobbering existing files. An export failure retains the successful asset/job and returns `export_failed` with recovery information.

Managed results live at `$XDG_DATA_HOME/gendock/assets/<prefix>/<uuid>/image.png`, with editing inputs alongside them; logs are in `$XDG_STATE_HOME/gendock/logs/image/`. `--out` selects an export path. Without it, export to `output.image_edit` for edits when configured, otherwise `output.image`, otherwise the invocation directory, as local-time `image_MMDD_HHMM.png`; collisions append `_2`, `_3`, etc. Explicit output collisions fail. JSON `export_path` identifies the actual filename.

SIGINT/SIGTERM kill/reap the owned component and cancel the job. Subsequent execution under the lock marks abandoned synchronous jobs interrupted only when no recorded worker remains alive. A surviving/reused PID blocks new work rather than being blindly killed. SIGKILL cannot be caught. Direct synchronous callers never claim queued jobs. Service tasks use the same verified execution core with existing job IDs, ownership heartbeats, cooperative preflight cancellation and managed-only server outputs. Graceful shutdown interrupts running work and preserves pending tasks; crashes conservatively reconcile surviving workers. See [server.md](server.md) for leases and notification semantics.

Discovery provides quick component/file-size/hardware checks, including `ready_operations`; full file hashes are verified at execution. Readiness is not a guarantee against later file changes or resource exhaustion.

Remote exports use the client's configured directory or invocation directory; server job metadata retains the managed asset rather than a foreign client filesystem path. `job fetch --out DIRECTORY` chooses `job_ID.png` with collision suffixes.

## Private components (development registration)

This machine reuses a verified isolated mflux 0.18.0 / MLX 0.31.1 environment. It is a development bridge; the automatic self-contained component installer remains pending. The private `$XDG_DATA_HOME/gendock/runtimes/image.json` registers absolute, fingerprinted entrypoints:

```json
{
  "adapter": "mflux",
  "version": "0.18.0",
  "executable": "/absolute/private/runtime/bin/mflux-generate-flux2",
  "executable_sha256": "sha256-of-generation-entrypoint",
  "edit_executable": "/absolute/private/runtime/bin/mflux-generate-flux2-edit",
  "edit_executable_sha256": "sha256-of-edit-entrypoint"
}
```

Older registrations without edit fields still support generation; editing reports `runtime_not_ready`. Model definitions cannot select executables. Fingerprints currently cover entrypoints, not every dependency. The future installer must verify/pin complete environments, activate atomically and support rollback. End users do not configure engine/Python flags.

Tests exercise configuration, no-download failures, source snapshots, input formats, output/export checks, cancellation and persistence using tiny fixtures. The [E2E scripts](testing.md) also chain service startup → async generate → wait → fetch → upload/edit → wait → fetch. An actual isolated host/client service run passed on M4 Pro / 48 GiB in 46.74 seconds: blue cup generation and green-cup editing, both 512×512, unchanged original bytes, changed decoded pixels and visually preserved composition. The client had no model installation. This validates one recipe, not all models or physical-network load.
