# Testing and end-to-end acceptance

Run from the repository root. Rust integration/E2E logic lives in `tests/`; `scripts/e2e*.sh` are thin Cargo launchers for selected suites. This follows common Rust project organization; there is no community rule requiring shell launchers to live in `tests/`. Ordinary checks use temporary XDG directories and tiny private worker executables. They do not download weights, require Python or invoke a GPU. `cargo test` reports passing/failing tests, not line coverage.

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked

# Focused E2E script: real CLI processes, HTTP and SQLite, tiny simulated inference
./scripts/e2e.sh

# Individual suites
cargo test --locked --test image
cargo test --locked --test server -- --nocapture
cargo test --locked --test queue
cargo test --locked --test server_lifecycle
```

`tests/image.rs` verifies the direct generation/editing flow, configuration, formats, unchanged sources and exact snapshots, atomic exports, worker failures and cancellation. `tests/server.rs` chains real commands for service startup, async generation, waiting, fetching, upload/editing and fetching again. It also verifies 22 separate caller identities submitting concurrently, FIFO order, one executing component, pending limits, queued/running cancellation, shutdown/restart, crash with a surviving worker, owned uploads/results, path rejection, live SSE delivery/reconnection and export-race recovery. These workers validate orchestration; they cannot establish model quality.

`tests/queue.rs` uses independent SQLite connections to race admission/claiming, verifies owner fences and checks a cancel/busy race does not strand a running task. File-hash tests verify both catalog checksum algorithms and cooperative cancellation without modifying model files. Existing storage suites verify initialization, preserved compatible data and refusal to reset incompatible schemas.

`tests/server_lifecycle.rs` checks default foreground readiness/shutdown, severity colors, absence of trailing success JSON, specific occupied-port errors in both modes and conflicting flags. It exercises unauthenticated 401, unknown-job 404, invalid-body 400, injected credential-store failure 500 and authenticated 200; verifies WARN/ERROR/INFO logs, request-ID correlation, both console/file output, and omission of tokens/query/body content. It also checks bare `--host` LAN binding, explicit `--host IP`, and actual unknown paths in 401/404 logs while query values remain omitted. Network unit tests cover loopback/wildcard/concrete bindings and IPv6 URL formatting without depending on a live LAN. Launchd unit tests round-trip escaped plist paths, verify environment isolation and owned cleanup, and exercise bootstrap rollback/foreign-file preservation. The explicit macOS OS-integration test performs real bootstrap/status/plutil/unregister with temporary HOME, and refuses to replace an existing `gendockd` job:

```sh
cargo test --locked --test launchd_live -- --ignored --nocapture
```

This tests launchd registration/start/removal, not an actual computer reboot/login.

## Actual model pipeline

This opt-in acceptance requires an already installed `flux2-klein-4b-q4` and a registered private generate/edit runtime on this Mac. It never downloads missing components. Readiness/setup: [image-generation.md](image-generation.md).

```sh
./scripts/e2e-real.sh

# Equivalent command; release avoids slow multi-GiB checks in debug builds
cargo test --release --locked --test real_image -- --ignored --nocapture
```

The test reads the existing model registration using a read-only SQLite connection. Optionally set `GENDOCK_E2E_MODEL_DIR` to an existing complete snapshot. It copies the runtime registration into an isolated temporary host XDG tree, references existing weights read-only, starts a daemon on a free loopback port and creates a separate client identity/state directory. It then runs:

1. `image generate --async` for a 512×512 blue ceramic cup, seed 42, four steps.
2. `job wait`, then `job fetch` into the client's generated directory.
3. `image edit --async` to change the cup to green, uploading the fetched image.
4. `job wait`, then `job fetch` into the client's edited directory.
5. PNG decoding, 512×512 dimensions, changed decoded pixel content, unchanged original bytes, input snapshot hash and absence of client model installation.

The service stops at the end. Test state, private test credentials, logs and images stay in a retained private temporary directory; the test prints `Real E2E artifacts: PATH`. `report.json` records image paths and hashes without tokens. Inspect the images for semantic quality: changed pixels alone cannot prove an instruction was followed. Remove the printed directory when its evidence is no longer needed.

The initial integrated real-service run passed on M4 Pro / 48 GiB in 46.74 seconds. Visual review confirmed a blue cup became green with the composition substantially preserved. This establishes one recipe on one host, not physical-network performance or a 22-person real-model load test.

If the process is forcibly killed, its cleanup guard cannot run. The retained host path identifies the isolated service state; stop that service using its XDG roots before discarding the directory. Routine test runs skip the real-model, real-launchd and network README tests.

The optional upstream transport check is separate and downloads only a pinned README:

```sh
cargo test --locked --test hf_live -- --ignored
```
