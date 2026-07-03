#!/usr/bin/env bash
set -u

if command -v nvidia-smi >/dev/null 2>&1; then
  nvidia-smi
else
  echo "nvidia-smi not found"
fi

rustc --version
cargo --version

if command -v vulkaninfo >/dev/null 2>&1; then
  vulkaninfo --summary
else
  echo "vulkaninfo not found; skipping optional Vulkan check"
fi

