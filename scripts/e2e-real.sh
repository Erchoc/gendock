#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
# Opt-in GPU test. Reuses installed model files and registered private components read-only.
# Service state, credentials, client state and outputs use a separate temporary XDG tree.
cargo test --release --locked --test real_image -- --ignored --nocapture
