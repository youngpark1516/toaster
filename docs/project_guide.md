# Toaster Project Guide

This document is the working snapshot of Toaster: what has been built, which
technologies it uses, how its pieces fit together, how to run it locally or on a
Slurm GPU node, and what should come next.

## Project purpose

Toaster is a Rust-based, headless path tracer and procedural scene sandbox. It
keeps a readable CPU reference renderer alongside a `wgpu` compute renderer,
supports JSON-authored scenes and animation, and can publish completed GPU frames
to a browser as an MJPEG stream.

The project is deliberately not a game engine, Blender replacement, desktop GUI,
or production renderer. The current emphasis is understandable rendering code,
portable GPU compute, remote-cluster operation, and measurable milestones.

## Current capabilities

- CPU and GPU rendering of spheres and triangles.
- Diffuse, metal, dielectric, and emissive materials.
- Sky, black, or equirectangular HDR/PNG/JPEG environment backgrounds.
- Emissive sphere and triangle area lights with direct-light sampling.
- JSON scene loading and validation.
- Camera and object-group translation and rotation tracks.
- Linear and step animation interpolation.
- Finite PNG animation rendering on the GPU.
- `.gltf` and `.glb` triangle geometry import.
- glTF node-transform evaluation and indexed geometry loading.
- Smooth-normal and UV interpolation at triangle hits.
- glTF base-color factors and 8-bit base-color textures in both renderers.
- Repeating bilinear texture filtering with sRGB-to-linear conversion.
- Linear floating-point environment radiance with intensity and yaw controls.
- Scene-relative mesh paths, Toaster material overrides, and animation groups.
- Headless MJPEG live preview with health and render-status endpoints.
- Finite or indefinite preview schedules, animation looping, Ctrl+C shutdown,
  render overrides, and adaptive samples-per-frame.
- Remote browser access through SSH port forwarding.
- Repeatable GPU benchmark reports with warmups, timing-stage summaries,
  reference-image hashes, and baseline comparison.
- Leveled text or JSON operational logging.

## Technology stack

| Component | Purpose |
| --- | --- |
| Rust 2021 | Workspace language and package model |
| `wgpu` 26 | Portable headless GPU compute and buffer management |
| WGSL | GPU path-tracing compute shader |
| `glam` | Vectors, quaternions, matrices, and transforms |
| `serde` / `serde_json` | Scene and animation JSON parsing |
| `gltf` | `.gltf`/`.glb` documents, buffers, accessors, and node traversal |
| `image` | HDR/PNG/JPEG loading, PNG output, and JPEG encoding |
| `axum` | Browser-facing HTTP routes |
| `tokio` | Server runtime, signals, tasks, and latest-value watch channels |
| `tokio-stream` | Adapting watch-channel updates into HTTP response streams |
| `clap` | Command-line parsing and validation |
| `bytemuck` | Safe conversion of Rust GPU structures to byte slices |
| `rand` | Sampling in the CPU reference renderer |
| `anyhow` | Context-rich application errors |
| `pollster` | Driving async GPU entry points from synchronous commands |
| `tracing` | Structured application, renderer, and HTTP lifecycle events |
| `sha2` | Stable hashes for benchmark reference frames |

There is no CUDA, OptiX, desktop window system, FFmpeg, WebRTC, RTMP, HLS, or
cloud orchestrator in the current design.

## Workspace layout

| Crate | Responsibility |
| --- | --- |
| `toaster-cli` | Commands, validation, render overrides, server/render lifecycle |
| `toaster-core` | Shared rays, cameras, colors, and image buffers |
| `toaster-scene` | Scene schema, validation, materials, objects, and animation |
| `toaster-cpu` | Readable reference integrator and primitive intersections |
| `toaster-gpu` | Device setup, GPU layouts, dispatch, readback, and frame sinks |
| `toaster-bvh` | BVH construction and flattened traversal data; currently in progress |
| `toaster-assets` | Indexed meshes and glTF/GLB geometry import |
| `toaster-server` | HTML, MJPEG, health, status, and latest-frame broadcasting |

The main data flow is:

```text
JSON scene ───────┐
environment map ─┼─> toaster-scene ─> CPU renderer ─> PNG
glTF asset ──────> toaster-assets ─┘
                         └───────> GPU renderer ─> RGBA frame
                                                    ├─> PNG
                                                    └─> JPEG ─> Axum ─> browser
```

Imported geometry is currently expanded into the same flat triangle list used
by both renderers. This keeps glTF import independent from the BVH implementation.

## Build and validation

Use a recent stable Rust toolchain:

```sh
cargo check --workspace
cargo test --workspace
cargo run -p toaster-cli -- info
```

Use release mode for performance measurements and live previews:

```sh
cargo build --release -p toaster-cli
```

Debug builds are useful for development but substantially exaggerate CPU-side
conversion and JPEG costs.

## CLI commands

### CPU still image

```sh
cargo run -p toaster-cli -- cpu-render \
  scenes/003_cornell_box.json \
  --out out/cornell.png \
  --width 800 --height 800 --samples 128 --max-bounces 8
```

The CPU renderer is intentionally simple and currently single-threaded.

### GPU still image

```sh
cargo run --release -p toaster-cli -- gpu-render \
  scenes/003_cornell_box.json --out out/cornell-gpu.png
```

### GPU PNG animation

Render by duration:

```sh
cargo run --release -p toaster-cli -- gpu-render \
  scenes/006_rotating_cube.json \
  --out out/cube.png --fps 24 --duration 2
```

Render an exact frame count:

```sh
cargo run --release -p toaster-cli -- gpu-render \
  scenes/006_rotating_cube.json \
  --out out/cube.png --fps 24 --frames 48
```

Animated outputs are numbered, such as `cube_0000.png` and `cube_0001.png`.

### Local live preview

```sh
cargo run --release -p toaster-cli -- stream-preview \
  scenes/009_gltf_textured_quad.json \
  --host 127.0.0.1 --port 7878 --fps 12 --loop-duration 8
```

Open <http://127.0.0.1:7878/>. Omit `--duration` to run until Ctrl+C, or add a
finite duration:

```sh
--duration 20
```

Preview frames remain in memory and are not written to disk.

### Preview quality overrides

```sh
cargo run --release -p toaster-cli -- stream-preview \
  scenes/007_mesh_long_preview.json \
  --fps 12 --width 600 --height 600 --samples 8 --max-bounces 6 \
  --loop-duration 120
```

Adaptive sampling changes only samples per frame and stays within explicit
bounds:

```sh
cargo run --release -p toaster-cli -- stream-preview \
  scenes/007_mesh_long_preview.json \
  --fps 12 --samples 8 --adaptive-samples \
  --min-samples 1 --max-samples 16 --loop-duration 120
```

If even one sample exceeds the frame budget, adaptive sampling cannot reach the
target FPS; lower resolution or bounce depth is then required.

### Static progressive preview

Progressive mode publishes an increasingly clean static image while preserving
the normal latest-frame streaming behavior:

```sh
cargo run --release -p toaster-cli -- stream-preview \
  scenes/010_environment_map.json \
  --progressive --batch-samples 2 --target-samples 256 --fps 12
```

The batch defaults to 1 spp and the target defaults to the scene sample count.
The browser and `/status` report both the latest batch and accumulated sample
counts. Once the target is exact, the GPU becomes idle while the server holds the
final image until Ctrl+C or the finite preview duration ends. Adaptive sampling
may adjust batch size and clamps its final batch to the remaining target.

Progressive mode rejects animation tracks, `--loop-duration`, and `--samples`;
use `--batch-samples` and `--target-samples` for its separate controls.

### Standalone server

```sh
cargo run -p toaster-cli -- server --host 127.0.0.1 --port 7878
```

This exercises the server independently. A live preview command is required to
produce rendered frames.

### GPU benchmark

Create a release-mode baseline before changing acceleration or traversal code:

```sh
cargo run --release -p toaster-cli -- benchmark scenes/004_mesh.json \
  --warmup 2 --runs 5 --out out/baseline.json \
  --image-out out/baseline.png
```

The report separates one-time setup from steady-state scene upload, dispatch,
readback, RGBA conversion, and total frame timing. Every benchmark frame uses
animation time and frame seed zero. Use `--compare` to print deltas and
`--max-regression-percent` to enforce a median-total limit on the same GPU.

All normal commands emit text logs to stderr at `info`. Global `--log-level`
and `--log-format json` flags or `RUST_LOG` enable detailed local or cluster
diagnostics. See [benchmarking.md](benchmarking.md) for the complete workflow.

## Browser endpoints

| Route | Response |
| --- | --- |
| `/` | Minimal preview page with image and current metrics |
| `/stream` | `multipart/x-mixed-replace` stream of complete JPEG frames |
| `/status` | Latest frame index, timing, FPS, dimensions, samples, and bounces |
| `/healthz` | Small HTTP 200 health response |

The server uses `tokio::sync::watch` as a latest-value broadcaster. Slow clients
miss superseded frames instead of creating queues or stalling the renderer.

## Streaming from a Slurm GPU node

When the GPU node is reachable only through a cluster login host, the connection
path is:

```text
local browser -> local SSH port -> login node -> allocated GPU node -> Toaster
```

First obtain a GPU allocation using the cluster's normal Slurm command. On the
allocated node, record its fully qualified name:

```sh
hostname -f
```

Start Toaster on the GPU node. It must listen on a non-loopback interface because
the forwarded connection arrives from the login node:

```sh
cargo run --release -p toaster-cli -- stream-preview \
  scenes/009_gltf_textured_quad.json \
  --host 0.0.0.0 --port 7878 --fps 12 --loop-duration 8
```

On the local computer, forward through the login host to the allocated node. For
the `zixian` allocation used during development:

```sh
ssh -N -L 7878:zixian.nodes.scc.sdsc.edu:7878 sc
```

Keep both processes running, then verify locally:

```sh
curl http://127.0.0.1:7878/healthz
```

Open <http://127.0.0.1:7878/>. If local port 7878 is occupied, map a different
local port while leaving Toaster's remote port unchanged:

```sh
ssh -N -L 8787:zixian.nodes.scc.sdsc.edu:7878 sc
```

Then open <http://127.0.0.1:8787/>. The preview currently has no authentication,
so bind `0.0.0.0` only on a trusted cluster network and stop it when finished.

## Scene format summary

A scene contains camera settings, render settings, named materials, objects, and
optional animation tracks. Existing examples under `scenes/` are the canonical
reference.

Supported materials:

- `diffuse`: `albedo`
- `metal`: `albedo` and `roughness`
- `dielectric`: index of refraction (`ior`)
- `emissive`: `color` and `strength`

Supported objects:

- `sphere`: center, radius, material, and optional group
- `triangle`: three vertices, material, and optional group
- `mesh`: relative or absolute glTF path, optional material override, and group

Example imported mesh:

```json
{
  "type": "mesh",
  "path": "../assets/models/tetrahedron.gltf",
  "material": "porcelain",
  "group": "tetrahedron"
}
```

glTF node transforms are applied during import. When `material` is present, that
Toaster material overrides every imported primitive. When it is omitted, Toaster
uses each primitive's glTF base-color factor and texture. Smooth `NORMAL` and
`TEXCOORD_0` attributes are interpolated barycentrically.

Render backgrounds accept the original `"sky"` and `"black"` strings or an
equirectangular environment description:

```json
"background": {
  "type": "environment",
  "path": "../assets/environments/studio_test.hdr",
  "intensity": 8.0,
  "rotation_degrees": 20.0
}
```

Environment paths are relative to the scene. HDR values stay as linear floats;
PNG and JPEG values are converted from sRGB. Both renderers bilinearly sample
the map when a ray misses.

Animation tracks can target the camera or every object sharing a group. Supported
tracks are translation and axis/pivot rotation with linear or step interpolation.

## Included scenes

| Scene | Demonstrates |
| --- | --- |
| `001_spheres.json` | Basic spheres |
| `002_ground.json` | Ground geometry |
| `002_materials.json` | Diffuse, metal, glass, and emission |
| `003_cornell_box.json` | Triangle room and area lighting |
| `004_mesh.json` | Animated hand-authored triangle mesh |
| `005_terrain.json` | Terrain scene |
| `006_rotating_cube.json` | Group rotation |
| `007_mesh_long_preview.json` | Lower-cost, long-running preview |
| `008_gltf_tetrahedron.json` | Imported and animated glTF geometry |
| `009_gltf_textured_quad.json` | Smooth normals and glTF base-color texture |
| `010_environment_map.json` | HDR environment background and reflections |

## Current limitations

- Triangle intersections remain a linear scan until BVH integration lands.
- Only glTF base-color factors and base-color textures are used; other PBR inputs
  are treated as diffuse.
- Only `TEXCOORD_0` and 8-bit base-color images are supported.
- Texture sampler modes are currently fixed to repeat with bilinear filtering.
- Texture alpha is loaded but ignored by shading.
- Only glTF triangle primitives are accepted.
- Skinning, morph targets, and non-triangle primitives are unsupported.
- GPU rendering reads each completed frame back to the CPU.
- Preview frames are JPEG images, not a modern low-latency video codec.
- There is no authentication or multi-render orchestration.
- The CPU renderer is single-threaded.
- Progressive accumulation is static-only; animated previews have no temporal
  accumulation, reprojection, or denoiser.

## Next milestone

Environment-map importance sampling is complete: CPU and GPU renderers share a
luminance-and-latitude distribution, directly sample the environment at diffuse
surfaces, and use the power heuristic to combine environment and BSDF paths
without bias.

After the separately developed BVH is integrated, the next major subsystem is
deterministic rigid-body physics. Denoising, expanded PBR material inputs, and
procedural scene generation remain later rendering and content milestones.
