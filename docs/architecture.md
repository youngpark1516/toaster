# Architecture

- `toaster-cli`: user-facing commands and process orchestration.
- `toaster-core`: shared rays, cameras, colors, and image buffers.
- `toaster-scene`: serializable scenes, objects, materials, transforms, and loading.
- `toaster-cpu`: readable reference integrator and intersections.
- `toaster-gpu`: `wgpu` device setup, buffers, compute pipeline, and readback.
- `toaster-bvh`: bounding boxes, hierarchy construction, and flattened layouts.
- `toaster-assets`: mesh representation and future glTF loading.
- `toaster-server`: lightweight HTML, health, and latest-frame MJPEG preview endpoints.

Dependencies point inward toward core data. Renderer-specific code stays out of scene descriptions, and GPU concerns remain isolated from the CPU reference implementation.

The live preview path keeps rendering and HTTP transport separate. `toaster-gpu` produces completed RGBA frames through a reusable sink, `toaster-cli` JPEG-encodes and publishes them, and `toaster-server` broadcasts only the latest JPEG through a Tokio watch channel. A separate latest-value status channel feeds `/status` and the browser metrics display. Slow browser clients therefore drop superseded frames instead of blocking the GPU render loop.
