# GPU Benchmarking and Logging

This is the operational benchmark guide. See the [documentation index](README.md)
for related design and usage guides.

Use Toaster's benchmark command to capture a baseline before changing renderer
acceleration code and a candidate afterward. Always use a release build and the
same allocated GPU, scene, resolution, sample count, and bounce depth.

## Create a baseline

```sh
cargo run --release -p toaster-cli -- benchmark scenes/004_mesh.json \
  --warmup 2 --runs 5 \
  --out out/baseline.json \
  --image-out out/baseline.png
```

`--warmup` defaults to 2 and may be zero. `--runs` defaults to 5 and must be
positive. The scene's render settings apply unless `--width`, `--height`,
`--samples`, or `--max-bounces` overrides them.

GPU setup is timed once. Warmup frames are discarded, then measured frames reuse
the same device, buffers, bind group, and pipeline. Every frame independently
renders the scene at animation time `0` with frame seed `0`; progressive
accumulation and frame pacing are not involved.

The JSON report contains:

- schema and Toaster versions plus a Unix timestamp;
- resolved scene settings and geometry/light counts;
- GPU name, backend, device type, driver, and driver information;
- setup time and every measured frame;
- min, mean, median, nearest-rank p95, and max for scene update/upload,
  dispatch plus GPU wait, readback/map, RGBA conversion, and total time;
- a SHA-256 hash of the final RGBA bytes and the optional reference-image path.

PNG and JSON writes happen after measurement. The SHA-256 value identifies the
result but is not a portable golden-image assertion: floating-point output may
vary between GPU drivers.

## Compare an optimization

```sh
cargo run --release -p toaster-cli -- benchmark scenes/004_mesh.json \
  --warmup 2 --runs 5 \
  --out out/candidate.json \
  --compare out/baseline.json
```

The command reports percentage changes for setup and each median timing stage.
Resolved render settings and geometry counts must match. A different adapter or
driver produces a warning because the timing delta may not isolate the code
change.

To use the comparison in a script or CI job, add a permitted regression:

```sh
--max-regression-percent 10
```

The command writes the candidate report and then exits unsuccessfully when its
median total frame time exceeds the limit. Threshold enforcement requires the
same GPU name, backend, and device type as the baseline.

## Structured logs

All operational logs are written to stderr, leaving explicit command output and
benchmark report files separate. Text at `info` is the default:

```sh
cargo run --release -p toaster-cli -- gpu-render scenes/004_mesh.json \
  --out out/mesh.png
```

Enable per-frame stage timings with a global CLI option:

```sh
cargo run --release -p toaster-cli -- --log-level debug \
  benchmark scenes/004_mesh.json --out out/debug.json
```

When `--log-level` is omitted, `RUST_LOG` can filter individual crates:

```sh
RUST_LOG=toaster_gpu=debug,toaster_server=info \
  cargo run --release -p toaster-cli -- stream-preview scenes/004_mesh.json
```

An explicit `--log-level` overrides `RUST_LOG`. For newline-delimited JSON logs
suitable for cluster collection, select JSON and redirect stderr:

```sh
cargo run --release -p toaster-cli -- --log-format json \
  benchmark scenes/004_mesh.json --out out/baseline.json \
  2>out/benchmark.log.jsonl
```

CLI log levels cover Toaster crates and HTTP request tracing without enabling
verbose dependency internals. Use a targeted `RUST_LOG` directive such as
`RUST_LOG=toaster_gpu=debug,wgpu_core=debug` when diagnosing wgpu itself.

On Slurm, keep the allocation, GPU type, loaded driver/modules, and benchmark
command identical between baseline and candidate runs. Avoid benchmarking on a
shared or thermally throttled GPU when interpreting small differences.

## Geometry-scaling suite

The checked-in [`scenes/benchmarks`](../scenes/benchmarks/) suite separates
triangle, sphere, mixed-geometry, and non-BVH environment costs. Its triangle
cases contain 128, 2,048, and 8,192 triangles over the same terrain footprint;
its sphere cases contain 64 and 512 subject spheres. All workloads are static
and use matched 320×180, 4-sample, 4-bounce settings.

Generate all pre-BVH reports inside a GPU allocation:

```sh
./scripts/run_benchmark_suite.sh pre-bvh
```

To capture the post-BVH comparison, run the same suite and enforce a 10% median
total-frame regression limit on the same adapter:

```sh
TOASTER_BENCH_COMPARE_DIR=out/benchmarks/pre-bvh \
TOASTER_BENCH_MAX_REGRESSION_PERCENT=10 \
  ./scripts/run_benchmark_suite.sh post-bvh
```

The runner fails if wgpu selects a CPU adapter. Preserve the ignored baseline
directory until comparison is complete. The environment control should remain
approximately stable; triangle and sphere scaling cases show traversal gains
and expose acceleration-structure overhead at small primitive counts.

The checked-in [RTX 2080 Ti pre-BVH reports](benchmarks/pre-bvh/rtx-2080-ti/)
provide the public linear-traversal baseline summarized in the root README.
