#!/usr/bin/env bash
set -euo pipefail

benchmark_label="${1:-pre-bvh}"
benchmark_root="${2:-out/benchmarks/${benchmark_label}}"
warmup_frames="${TOASTER_BENCH_WARMUP:-2}"
measured_frames="${TOASTER_BENCH_RUNS:-5}"
comparison_root="${TOASTER_BENCH_COMPARE_DIR:-}"
regression_limit="${TOASTER_BENCH_MAX_REGRESSION_PERCENT:-}"

if [[ -n "${regression_limit}" && -z "${comparison_root}" ]]; then
    echo "TOASTER_BENCH_MAX_REGRESSION_PERCENT requires TOASTER_BENCH_COMPARE_DIR" >&2
    exit 2
fi

if ! command -v grep >/dev/null 2>&1; then
    echo "benchmark suite requires grep to validate the selected adapter" >&2
    exit 127
fi

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repository_root}"
mkdir -p "${benchmark_root}"
cargo build --release -p toaster-cli

benchmark_scenes=(
    scenes/benchmarks/001_triangles_128.json
    scenes/benchmarks/002_triangles_2048.json
    scenes/benchmarks/003_triangles_8192.json
    scenes/benchmarks/004_spheres_64.json
    scenes/benchmarks/005_spheres_512.json
    scenes/benchmarks/006_mixed_2048t_128s.json
    scenes/benchmarks/007_environment_control.json
)

for scene_path in "${benchmark_scenes[@]}"; do
    scene_name="$(basename "${scene_path}" .json)"
    report_path="${benchmark_root}/${scene_name}.json"
    command=(
        target/release/toaster benchmark "${scene_path}"
        --warmup "${warmup_frames}"
        --runs "${measured_frames}"
        --out "${report_path}"
    )

    if [[ -n "${comparison_root}" ]]; then
        command+=(--compare "${comparison_root}/${scene_name}.json")
        if [[ -n "${regression_limit}" ]]; then
            command+=(--max-regression-percent "${regression_limit}")
        fi
    fi

    echo "Benchmarking ${scene_name}" >&2
    "${command[@]}"
    if grep -Eq '"device_type"[[:space:]]*:[[:space:]]*"Cpu"' "${report_path}"; then
        echo "benchmark selected a CPU adapter; rerun inside a GPU allocation" >&2
        exit 1
    fi
done

echo "Benchmark suite reports written to ${benchmark_root}" >&2
