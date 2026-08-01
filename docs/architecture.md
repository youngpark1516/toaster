# Architecture

- `toaster-cli`: user-facing commands and process orchestration.
- `toaster-core`: shared rays, cameras, colors, and image buffers.
- `toaster-scene`: serializable scenes, objects, materials, transforms, and loading.
- `toaster-cpu`: readable reference integrator and intersections.
- `toaster-gpu`: `wgpu` device setup, buffers, compute pipeline, and readback.
- `toaster-bvh`: bounding boxes, hierarchy construction, and flattened layouts.
- `toaster-assets`: indexed mesh representation and glTF/GLB geometry loading.
- `toaster-server`: lightweight HTML, health, and latest-frame MJPEG preview endpoints.

Dependencies point inward toward core data. Renderer-specific code stays out of scene descriptions, and GPU concerns remain isolated from the CPU reference implementation.

glTF import stops at renderer-independent indexed geometry, vertex attributes,
and base-color image data. `toaster-scene` resolves mesh paths relative to the
scene document, applies an optional Toaster material override, and expands
imported indices into the flat triangle list currently shared by the CPU and GPU
renderers. Smooth normals and UVs occupy a parallel array keyed by triangle index,
so BVH traversal can retain the existing position/material structure and retrieve
shading attributes only after a hit.

The live preview path keeps rendering and HTTP transport separate. `toaster-gpu` produces completed RGBA frames through a reusable sink, `toaster-cli` JPEG-encodes and publishes them, and `toaster-server` broadcasts only the latest JPEG through a Tokio watch channel. A separate latest-value status channel feeds `/status` and the browser metrics display. Slow browser clients therefore drop superseded frames instead of blocking the GPU render loop.
