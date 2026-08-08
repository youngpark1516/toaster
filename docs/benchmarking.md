# Benchmarking

Use release builds and keep the GPU, driver, scene, resolution, samples, and
bounce depth fixed between comparisons.

## One scene

```sh
cargo run --release -p toaster-cli -- benchmark scenes/004_mesh.json \
  --warmup 2 --runs 5 --out out/mesh.json --image-out out/mesh.png
```

Setup is timed once. Warmups are discarded, and measured frames reuse the
device, pipeline, and buffers. The JSON report records the adapter and driver,
resolved workload, every frame, summary statistics for each stage, and a SHA-256
hash of the final RGBA output. The report separates animation evaluation,
physics evaluation, neutral geometry updates, light-list rebuild, BVH rebuild,
GPU upload, dispatch/wait, readback, conversion, output delivery, and total
frame time. The final `--image-out` and JSON report writes occur after frame
measurement.

Compare a candidate with `--compare out/baseline.json`. Add
`--max-regression-percent 10` when a script should fail on a median total-frame
regression. Adapter mismatches produce a warning; regression enforcement
requires the same GPU name, backend, and device type.

## Geometry-scaling suite

The seven workloads in [`scenes/benchmarks`](../scenes/benchmarks/) cover three
triangle counts, two sphere counts, mixed geometry, and a non-BVH environment
control. All use 320×180, 4 samples, 4 bounces, 2 warmups, and 5 measured frames.

```sh
./scripts/run_benchmark_suite.sh post-bvh
```

The runner rejects CPU adapters. To compare against another report directory:

```sh
TOASTER_BENCH_COMPARE_DIR=docs/benchmarks/pre-bvh/rtx-2080-ti \
  ./scripts/run_benchmark_suite.sh post-bvh
```

The root [README](../README.md#bvh-performance) summarizes the measured result.
Raw RTX 2080 Ti reports are checked in for both
[pre-BVH](benchmarks/pre-bvh/rtx-2080-ti/) and
[post-BVH](benchmarks/post-bvh/rtx-2080-ti/). Their output hashes match for every
workload. The large cases show the traversal benefit; small cases expose fixed
BVH overhead, and the 8,192-triangle case shows that BVH update/upload is now a
meaningful part of frame time.

## Logging and cluster practice

Logs go to stderr. Use global `--log-level debug` for stage timings or
`--log-format json` for newline-delimited cluster logs. `RUST_LOG` can target a
crate when those flags are omitted. On Slurm, use the same allocation type and
software environment, avoid a shared or throttled GPU, and retain the raw JSON
reports with the command and commit being measured.

For moving-physics diagnostics, render a short sequence with debug logs. Unlike
the static benchmark command, this advances fixed physics ticks and therefore
shows the MVP's per-frame BVH rebuild cost:

```sh
mkdir -p out

cargo run --release -p toaster-cli -- --log-level debug --log-format json \
  gpu-render scenes/011_physics_rigid_bodies.json \
  --out out/physics_diagnostic.png --fps 24 --frames 24 \
  2>out/physics_diagnostic.jsonl
```

Each `completed GPU frame` record contains
`animation_evaluation_ms`, `physics_evaluation_ms`, `geometry_update_ms`,
`light_rebuild_ms`, `bvh_rebuild_ms`, `gpu_upload_ms`, `dispatch_wait_ms`,
`readback_ms`, `conversion_ms`, `output_ms`, and `total_ms`. Real-time preview
uses sequential frame indices even when rendering misses its requested deadline;
it lowers effective output FPS rather than skipping simulation states.
