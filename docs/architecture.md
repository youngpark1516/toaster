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

GPU animation readback has two output paths over the same render loop. The PNG
path writes each completed frame as a numbered image. The MP4 path converts the
same floating-point pixels to RGBA8 and streams them to an FFmpeg child process;
FFmpeg performs H.264 encoding and container finalization without temporary PNGs.

glTF import stops at renderer-independent indexed geometry, vertex attributes,
and base-color image data. `toaster-scene` resolves mesh paths relative to the
scene document, applies an optional Toaster material override, and expands
imported indices into the flat triangle list currently shared by the CPU and GPU
renderers. Smooth normals and UVs occupy a parallel array keyed by triangle index,
so BVH traversal can retain the existing position/material structure and retrieve
shading attributes only after a hit.

Environment images are decoded in `toaster-scene` into renderer-neutral linear
floating-point radiance. The CPU integrator samples that shared representation;
the GPU uploader copies the same pixels into a read-only storage buffer. CPU and
WGSL use the same latitude-longitude mapping, horizontal wrap, vertical clamp,
bilinear filtering, intensity, and yaw rotation.

The live preview and video export paths keep rendering and transport separate.
`toaster-gpu` produces completed RGBA frames through a reusable sink. The CLI
either JPEG-encodes and publishes them through `toaster-server`, or sends their
RGBA8 bytes directly to FFmpeg. A separate latest-value status channel feeds
`/status` and the browser metrics display. Slow browser clients therefore drop
superseded frames instead of blocking the GPU render loop.

Static progressive preview reuses the path tracer's read/write output buffer as
a persistent linear-color running average. Each dispatch supplies its previous
sample count through the render uniform, and the completed-frame sink reports
both batch and accumulated counts. No extra storage binding or CPU-side float
accumulation is required. Independent PNG, video, and animated-preview frames set
the previous count to zero and retain their existing behavior.
