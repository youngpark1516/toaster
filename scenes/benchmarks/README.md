# Benchmark scenes

These static scenes form Toaster's deterministic GPU performance suite. They share 320×180 output, 4 samples per pixel, 4 maximum bounces, and a fixed camera/material setup unless the workload requires an environment. The benchmark command evaluates time zero with frame seed zero.

| Scene | Geometry after loading | Purpose |
| --- | ---: | --- |
| `001_triangles_128.json` | 128 triangles, 1 light sphere | Small triangle control where acceleration overhead may dominate |
| `002_triangles_2048.json` | 2,048 triangles, 1 light sphere | Medium triangle traversal workload |
| `003_triangles_8192.json` | 8,192 triangles, 1 light sphere | Large triangle scaling workload |
| `004_spheres_64.json` | 64 subject spheres, 1 light sphere, 2 ground triangles | Small analytic-sphere workload |
| `005_spheres_512.json` | 512 subject spheres, 1 light sphere, 2 ground triangles | Large analytic-sphere workload |
| `006_mixed_2048t_128s.json` | 2,048 triangles, 128 subject spheres, 1 light sphere | Representative mixed traversal workload |
| `007_environment_control.json` | 4 spheres, environment map | Non-BVH control for lighting and conversion regressions |

The terrain meshes contain smooth analytic height variation so primary, bounce, and shadow rays exercise the geometry rather than missing an edge-on plane. Small, medium, and large triangle cases use the same footprint and shading; only tessellation changes.

Regenerate the checked-in JSON and embedded-buffer glTF fixtures with:

```sh
python3 scripts/generate_benchmark_scenes.py
```

Run the full pre-BVH suite inside a GPU allocation:

```sh
./scripts/run_benchmark_suite.sh pre-bvh
```

Reports are written under ignored `out/benchmarks/pre-bvh`. Preserve that directory until the matching post-BVH run is complete. See [GPU benchmarking and logging](../../docs/benchmarking.md) for comparison controls and interpretation.
