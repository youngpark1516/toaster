# Roadmap

## Phase 1: CPU reference renderer

Define rays, cameras, spheres, diffuse materials, sampling, accumulation, and PNG output. Favor clarity and tests over speed.

## Phase 2: GPU renderer

Port sphere tracing to a `wgpu` compute shader, add device setup and image readback, and compare results against the CPU renderer.

## Phase 3: Meshes and BVH

Add triangles, construct a CPU BVH, flatten it into GPU-friendly buffers, and measure traversal performance.

## Phase 4: Assets and procedural scenes

Import glTF meshes, add textures and environment lighting, then build reusable procedural scene generators.

## Phase 5: Preview server

Serve render status and images through a small browser-facing service without turning Toaster into a full editor.

