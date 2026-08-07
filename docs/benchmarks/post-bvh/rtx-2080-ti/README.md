# RTX 2080 Ti post-BVH results

These reports were generated from commit
`df714ccb15a2910a20a69de782ab7bc72da53698` on 2026-08-06 with:

```sh
TOASTER_BENCH_COMPARE_DIR=docs/benchmarks/pre-bvh/rtx-2080-ti \
  ./scripts/run_benchmark_suite.sh post-bvh
```

The adapter was an NVIDIA GeForce RTX 2080 Ti using Vulkan and NVIDIA driver
610.43.02. Each workload uses 320×180 output, 4 samples per pixel, 4 bounces, 2
warmup frames, and 5 measured frames. All seven output hashes match the pre-BVH
baseline.
