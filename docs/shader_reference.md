# Shader contracts

`toaster-gpu` compiles a diagnostic gradient shader and the WGSL path tracer.
Rust performs the shared exposure, ACES fitted tone mapping, linear-to-sRGB,
and RGBA8 conversion after readback. Display settings never alter shader-side
path tracing or linear-HDR accumulation.

## Bindings

Shader-facing Rust records live in `toaster_gpu::gpu_types` and use `#[repr(C)]`,
explicit padding, and `bytemuck::Pod`. Rust and WGSL layouts must change together.

| Binding | Access | Contents |
| --- | --- | --- |
| 0 | read/write storage | linear-HDR output |
| 1 | uniform | render parameters and resource counts |
| 2 | uniform | evaluated camera |
| 3 | read storage | spheres |
| 4 | read storage | materials |
| 5 | read storage | triangles |
| 6 | read storage | emissive lights |
| 7 | read storage | packed RGBA8 texture atlas |
| 8 | read storage | triangle normals and UVs |
| 9 | read storage | environment RGB and CDF |
| 10 | read storage | flattened BVH nodes |
| 11 | read storage | BVH primitive references |

Empty arrays use one-element dummy buffers; explicit counts prevent sentinel
reads. Animation buffers are allocated for their maximum required size.

## Invariants

- Workgroups are 8×8. Bounds checks protect partial edge groups.
- Each pixel stores `vec4<f32>` linear HDR. Progressive batches use the running
  average contract described in [Architecture](architecture.md).
- Each invocation owns RNG state derived from pixel and frame indices. Benchmarks
  fix the frame index; progressive batches advance it.
- The flattened BVH uses tagged leaf references for spheres and triangles. An
  empty hierarchy falls back to linear traversal.
- Sphere and two-sided triangle tests retain the closest hit and offset spawned
  rays to limit self-intersection.
- Diffuse, metal, dielectric, and emissive materials share an iterative bounce
  loop. Direct emissive-primitive and environment sampling use MIS.
- Environment evaluation, importance sampling, PDF lookup, intensity, and yaw
  rotation use the same latitude-longitude convention.
- Output is read back and converted once before PNG, MJPEG, video, or benchmark
  consumers receive it.

## Change checklist

1. Update Rust and WGSL layouts together, including padding and binding order.
2. Update buffer creation, bind-group layout, and bind-group entries.
3. Confirm animation uploads cannot exceed allocated buffer sizes.
4. Run workspace tests, strict Clippy, and `./scripts/check_docs.sh`.
5. Create a GPU pipeline and render a small scene; shader parsing occurs there.
