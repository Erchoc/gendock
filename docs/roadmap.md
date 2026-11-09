# Roadmap

## Current status

[功能进度与完成度](features.md) is the canonical feature-status table. It records per-feature readiness percentages, verified scope, blockers and acceptance criteria; update it in the same PR as capability changes. This roadmap defines development order, not a second set of progress estimates.

Local model management and FLUX.2 generation/editing are usable directly and through the authenticated shared-device service. FIFO consumption, async commands, cancellation/recovery and SSE completion now work. Automated acceptance covers 22 distinct caller identities with tiny workers and one actual-model remote generation/editing pipeline; physical-network and sustained real-model load testing remain open.

## Shared-device service: hardening and expanded acceptance

A resident service, basic scheduling and completion notifications are core requirements: 20+ colleagues may submit work from other computers to one shared Mac. Keep SQLite small; this requirement does not imply a large multi-tenant database design.

1. Extend the implemented service/API beyond loopback acceptance: physical clients, network interruption, sustained workloads, resource pressure, external HTTPS deployment and clearer diagnostics.
2. Add submission idempotency, credential rotation/revocation, retained-byte quotas and upload/asset/log cleanup. Preserve the small schema and existing task ownership boundaries.
3. Improve queue observability, richer progress and fair-share policy; retain FIFO and one GPU execution as defaults.
4. SSE current snapshots and polling fallback are implemented. Add persisted webhook delivery/retry state only when needed; historical event replay remains deferred.
5. Consider persistent model workers to reduce reload/switch overhead; the resident HTTP server currently starts a component per task.

## Global settings

`gendock config` now manages model defaults, remote connection preferences, media export directories and macOS task-scoped sleep prevention; see [configuration.md](configuration.md). Full Chinese/English help and message localization remains planned. Connect video/audio/music directory preferences as their execution adapters become available. Keep global preferences tied to observable behavior and stable JSON contracts.

## Runtime packaging and model coverage

- Private pinned execution-component installation, verification, rollback and diagnostics. Never use user Python. Existing local inference reuses a verified private environment; this is not a fresh-machine installer yet.
- Extend recipes and hardware checks for other models; validate recommendations through real runs.
- Improve available-memory checks and download progress.

## Later

- Versioned database upgrades after the 1.0 baseline is released.
- LTX video adapter, audio and music adapters.
- Additional remote task types and optional MCP sharing the implemented service core.
- Signed macOS distribution and release automation.
- Optional model publishing/upload workflows (public R2 URLs already work for download).
- Cloud compute/storage adapters and client synchronization.
- Desktop/App clients; Linux and Windows after macOS.

Features here are intentions, not availability claims. Nonfunctional operations must return explicit not-implemented errors.
