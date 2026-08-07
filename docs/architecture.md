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

## Engineering decisions

### Keep host and shader storage layouts explicit

GPU-facing Rust records use `#[repr(C)]`, explicit padding, and `bytemuck::Pod`.
Their field order and the bind-group order are a contract with the corresponding
WGSL structures and bindings; a change on either side requires a matching change
on the other. Geometry, materials, lights, texture pixels, triangle attributes,
and environment data use flat storage buffers so the shader can index them
directly. Logical empty arrays receive a one-element dummy allocation because
`wgpu` does not permit zero-sized buffer bindings, while explicit element counts
prevent the shader from reading the sentinel.

This design makes buffer sizes and transfers predictable and keeps the eventual
BVH representation compatible with GPU storage buffers. The cost is more manual
layout discipline than a higher-level GPU object model would require.

### Accumulate progressive samples in the output buffer

Static progressive preview treats the read/write linear-HDR output buffer as a
persistent running average. Each dispatch receives the number of samples already
represented by that buffer and combines them with the current batch:

```text
new_average = (old_average * old_samples + batch_average * batch_samples)
              / (old_samples + batch_samples)
```

The host clamps the last batch so accumulation reaches the requested target
exactly. Animation is rejected because averaging samples from different poses
would not converge to a meaningful image. This contract avoids another GPU
binding and CPU-side float accumulation while preserving independent-frame
behavior by setting the prior sample count to zero.

### Prefer portable compute and one output boundary

Toaster uses `wgpu` and WGSL instead of CUDA so the renderer is not tied to one
GPU vendor or one deployment backend. The same headless compute path can select
Vulkan, Metal, Direct3D, or OpenGL-compatible adapters through `wgpu`, which is a
better fit for cluster experiments and a future browser-facing project. The
tradeoff is giving up CUDA-specific profiling, libraries, and vendor-tuned ray
tracing features at this stage.

All GPU outputs cross one explicit boundary: render to linear floating-point
storage, copy to a map-readable buffer, read back on the CPU, and convert once to
RGBA8. PNG, MJPEG, MP4, and benchmark consumers share that conversion. FFmpeg
receives raw RGBA bytes over stdin and performs H.264 encoding without temporary
PNGs; it does not consume GPU memory directly.

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
