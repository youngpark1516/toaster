#!/usr/bin/env bash
set -euo pipefail

export RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -D warnings"

cargo doc --workspace --no-deps --document-private-items
cargo test --doc --workspace
