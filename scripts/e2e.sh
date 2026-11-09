#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
# Real CLI + tiny private workers. No weights, network downloads or Python required.
cargo test --locked --test image --test server --test server_lifecycle --test queue -- --nocapture
