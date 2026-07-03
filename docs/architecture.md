# Architecture

- `toaster-cli`: user-facing commands and process orchestration.
- `toaster-core`: shared rays, cameras, colors, and image buffers.
- `toaster-scene`: serializable scenes, objects, materials, transforms, and loading.
- `toaster-cpu`: readable reference integrator and intersections.
- `toaster-gpu`: `wgpu` device setup, buffers, compute pipeline, and readback.
- `toaster-bvh`: bounding boxes, hierarchy construction, and flattened layouts.
- `toaster-assets`: mesh representation and future glTF loading.
- `toaster-server`: future lightweight preview endpoints.

Dependencies point inward toward core data. Renderer-specific code stays out of scene descriptions, and GPU concerns remain isolated from the CPU reference implementation.

