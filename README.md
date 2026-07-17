# Toaster

Toaster is a Rust-based headless GPU path tracer and procedural 3D scene sandbox. It begins with a small CPU reference renderer, then uses `wgpu` compute shaders so the same project can run on cluster GPUs and portable graphics backends.

## Goals

- Learn path tracing from a clear CPU implementation.
- Build a modular GPU renderer with measurable milestones.
- Support meshes, glTF, procedural environments, and eventually browser previews.
- Keep the repository approachable for new contributors and suitable for a future public mirror.

## Non-goals

Toaster is not a Blender replacement, game engine, or production DCC application. It deliberately avoids starting with CUDA, OptiX, or Vulkan ray-tracing extensions.

## Quickstart

Install a recent stable Rust toolchain, then run:

```sh
cargo check
cargo run -p toaster-cli -- info
cargo run -p toaster-cli -- cpu-render scenes/003_cornell_box.json --out out/cornell.png
```

The CPU renderer supports spheres, triangles, diffuse/metal/glass materials, and emissive area lights. Higher sample counts produce cleaner images but take longer in the current single-threaded reference renderer.

### GPU animation

```sh
# 24 frames sampled over one second
cargo run -p toaster-cli -- gpu-render scenes/006_rotating_cube.json \
  --out out/cube.png --fps 24 --duration 1

# Exactly 48 frames at an explicitly selected frame rate
cargo run -p toaster-cli -- gpu-render scenes/006_rotating_cube.json \
  --out out/cube.png --fps 30 --frames 48

# A two-second rotating cube with an independently orbiting Cornell-room camera
cargo run -p toaster-cli -- gpu-render scenes/004_mesh.json \
  --out out/cornell-cube.png --fps 24 --duration 2
```

Multi-frame output is numbered from `cube_0000.png`. Omitting all timing flags renders one frame at time zero; partial or conflicting timing options are rejected rather than defaulted.

### Render setting overrides

Scene render settings are used by default. Override any combination of resolution, samples, and path depth from the command line:

```sh
cargo run -p toaster-cli -- cpu-render scenes/003_cornell_box.json \
  --out out/cornell.png \
  --samples 512 \
  --width 800 \
  --height 800 \
  --max-bounces 12

# A faster preview that keeps the scene's max-bounces setting:
cargo run -p toaster-cli -- cpu-render scenes/003_cornell_box.json \
  --out out/cornell-preview.png --width 400 --height 400 --samples 32
```

## Milestones

1. ✅ CPU sphere path tracer and material scattering
2. ✅ Emissive materials, initial triangles, and Cornell box
3. `wgpu` compute renderer and image readback
4. Triangle meshes and BVH acceleration
5. glTF assets, procedural scenes, and browser preview

See [the roadmap](docs/roadmap.md) and [architecture](docs/architecture.md) for more detail.
