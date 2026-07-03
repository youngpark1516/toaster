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
cargo run -p toaster-cli -- cpu-render scenes/001_spheres.json --out out/test.png
```

The render commands are placeholders in this setup milestone and do not yet write images.

## Milestones

1. CPU sphere path tracer
2. `wgpu` compute renderer and image readback
3. Triangle meshes and BVH acceleration
4. glTF assets and procedural scenes
5. Lightweight browser preview server

See [the roadmap](docs/roadmap.md) and [architecture](docs/architecture.md) for more detail.
