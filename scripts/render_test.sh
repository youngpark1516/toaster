#!/usr/bin/env bash
set -euo pipefail

cargo run -p toaster-cli -- info
cargo run -p toaster-cli -- cpu-render scenes/001_spheres.json --out out/test.png

