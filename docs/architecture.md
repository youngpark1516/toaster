# Architecture

Toaster loads JSON, glTF/GLB geometry, textures, and environment maps into a
renderer-independent scene. The CPU integrator provides a readable reference
path. The `wgpu` integrator uploads the evaluated scene and flattened BVH,
dispatches WGSL compute work, and returns the result through one shared readback
and RGBA conversion path. The root [README](../README.md) diagrams this flow.

The workspace keeps scene and asset code independent from renderer details:
`toaster-core` and `toaster-scene` hold shared data, `toaster-cpu` and
`toaster-gpu` implement the renderers, `toaster-bvh` builds acceleration data,
and the CLI/server crates own presentation and transport.

## Engineering decisions

### Explicit host/shader layouts

GPU-facing Rust records use `#[repr(C)]`, explicit padding, and
`bytemuck::Pod`. Field order, buffer element types, and the 12-entry binding
order are a contract with WGSL. Geometry, shading attributes, materials,
lights, textures, environment data, BVH nodes, and primitive references are
flat arrays so shaders can index them directly.

Because `wgpu` rejects zero-sized bindings, empty logical arrays receive a
one-element dummy allocation. Explicit counts ensure the shader never reads the
sentinel. This is more manual than a higher-level object model, but makes
alignment, transfer sizes, and CPU/GPU BVH agreement auditable.

### Progressive accumulation is a host/shader contract

Static progressive preview keeps a linear-HDR running average in the output
buffer. The host supplies the prior sample count, and each dispatch combines it
with the current batch:

```text
new_average = (old_average * old_samples + batch_average * batch_samples)
              / (old_samples + batch_samples)
```

The host clamps the final batch to reach the target exactly. Animation is
rejected because averaging different scene poses would not converge to one
image. Independent frames use a prior count of zero.

### Portable compute with one output boundary

Toaster uses `wgpu` rather than CUDA to support multiple GPU vendors and native
backends. This fits cluster experiments and future browser-facing work, at the
cost of CUDA-specific tooling and vendor libraries.

Every GPU output follows the same boundary: linear floating-point storage,
readback, then one RGBA8 conversion. PNG, MJPEG, video, and benchmark paths
therefore agree on pixel conversion. FFmpeg consumes raw RGBA frames over stdin,
avoiding temporary PNGs; this is not direct GPU-memory encoding.
