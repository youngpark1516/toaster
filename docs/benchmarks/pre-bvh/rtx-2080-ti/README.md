# RTX 2080 Ti pre-BVH baseline

These reports are the raw data behind the performance table in the root README.
They were generated from commit `d147e54` on 2026-08-03 with:

```sh
./scripts/run_benchmark_suite.sh pre-bvh
```

The selected adapter was an NVIDIA GeForce RTX 2080 Ti using the Vulkan backend
and NVIDIA driver 610.43.02. Each workload uses 320×180 output, 4 samples per
pixel, 4 maximum bounces, 2 discarded warmup frames, and 5 measured frames. The
benchmark fixes animation time and frame seed at zero and reuses one GPU setup
across warmup and measured frames.

The renderer used brute-force primitive traversal. These reports are intended as
the pre-BVH comparison point, not as general hardware rankings.
