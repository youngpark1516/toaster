# Shader reference

Toaster currently compiles two WGSL compute shaders through `toaster-gpu`: the diagnostic gradient and the path tracer. `pathtrace_triangles.wgsl` and `tonemap.wgsl` are reserved placeholders and are not loaded or compiled. Triangle tracing lives in the active path-tracing shader; tonemapping and quantization happen once in Rust after readback.

## Host and shader layout contract

All shader-facing Rust records are `#[repr(C)]`, `bytemuck::Pod`, and defined in `toaster_gpu::gpu_types`. Buffer fields and binding order must change in lockstep with WGSL. Padding fields are explicit so Rust sizes satisfy WGSL uniform/storage alignment.

| Rust host type | WGSL type | Purpose |
| --- | --- | --- |
| `GpuRenderParams` | `RenderParams` | dimensions, sample/depth controls, resource/BVH counts, frame seed, background/environment settings, prior progressive samples |
| `GpuCamera` | `Camera` | origin and image-plane basis |
| `GpuSphere` | `Sphere` | center/radius/material index |
| `GpuTriangle` | `Triangle` | three positions/material index |
| `GpuTriangleAttributes` | `TriangleAttributes` | optional normals/UVs and flags |
| `GpuMaterial` | `Material` | material tag, texture atlas metadata, albedo, roughness/IOR/emission |
| `GpuLight` | `Light` | emissive primitive geometry and cumulative surface area |
| `GpuBvhNode` | `BvhNode` | bounds plus leaf range or interior right-child index |
| `GpuPrimitiveRef` | `PrimitiveRef` | tagged sphere/triangle reference stored in leaf order |

The active render pipeline uses one bind group:

| Binding | Address space/access | Contents | Lifetime |
| --- | --- | --- | --- |
| 0 | storage, read/write | linear HDR output pixels | persistent across progressive batches; copied after every dispatch |
| 1 | uniform | render parameters | updated each frame |
| 2 | uniform | evaluated camera | updated each frame |
| 3 | storage, read | spheres | allocated once, updated for animation |
| 4 | storage, read | materials | immutable for a render session |
| 5 | storage, read | triangles | allocated once, updated for animation |
| 6 | storage, read | sampleable emissive lights | allocated once, updated for animation |
| 7 | storage, read | packed RGBA8 texture atlas | immutable |
| 8 | storage, read | parallel triangle shading attributes | allocated once, updated for animation |
| 9 | storage, read | environment RGB plus normalized CDF | immutable |
| 10 | storage, read | flattened BVH nodes | allocated for the full tree bound; updated for animation |
| 11 | storage, read | BVH primitive references | allocated once; updated for animation |

Empty logical arrays receive a one-element dummy buffer because wgpu bindings cannot have zero size. Shader loops use the explicit counts and never read those sentinels.

## Dispatch and output

Both shaders use `@workgroup_size(8, 8, 1)`. Rust dispatches `ceil(width / 8)` by `ceil(height / 8)` workgroups. The entry point rejects invocation IDs beyond the image edge and maps `(x,y)` to `y * width + x`.

The path tracer stores a `vec4<f32>` per pixel. For an independent frame it writes the batch mean. For progressive mode, `accumulated_samples` describes the old output and the shader applies the linear-HDR weighted running average:

```text
new_average = (old_average * old_samples + batch_average * batch_samples)
              / (old_samples + batch_samples)
```

The final batch is clamped by Rust so the new count reaches the target exactly. The buffer is copied to a map-readable buffer, interpreted as `[f32; 4]`, converted to the shared RGBA8 image, then passed to PNG, JPEG, MP4, preview, or benchmark consumers. This ensures every output path uses the same conversion.

## RNG and frame identity

Each invocation owns a private `rng_state`. `main` seeds it from the pixel index and `frame_index`, using `pcg_hash`; each `random_f32` call hashes the state again. `random_unit_vector` maps two draws uniformly to the sphere. Animated frames advance the seed. Benchmark mode fixes `frame_index` at zero so repeated frames are comparable. Progressive batches use successive frame indices, adding new sample patterns to the running average.

## Geometry and intersections

`hit_sphere` solves the analytic quadratic and chooses the nearest root beyond
`MIN_DISTANCE`. `hit_triangle` implements a two-sided Möller–Trumbore-style test,
rejects parallel/out-of-range hits, interpolates optional vertex normals and UVs,
and orients the normal against the incoming ray. `calc_intersections` traverses
the flattened BVH with an explicit stack and dereferences tagged leaf primitives;
the linear implementation remains as an empty-hierarchy fallback.

`MIN_DISTANCE` offsets continuation and shadow rays to reduce self-intersection. `MAX_DISTANCE` is the finite miss/shadow bound. A no-hit record uses maximum finite `f32` distance.

## Materials and path integration

`ray_color` iterates no more than `max_bounces`:

- Diffuse: evaluates constant/imported textured albedo, adds explicit emissive-primitive and environment estimates, then draws a cosine-like `normal + random_unit_vector` continuation.
- Metal: reflects, perturbs by clamped roughness, and rejects directions below the surface.
- Dielectric: uses Snell refraction, total internal reflection, and Schlick reflectance.
- Emissive: adds emission only when it has not already been accounted for by direct-light sampling, then terminates.

The current implementation intentionally has no Russian roulette. Paths terminate by miss, emissive hit, invalid scatter, or maximum depth.

## Direct lighting and MIS

Rust builds one area-weighted cumulative distribution over positive-emission triangles and spheres. `sample_light` chooses a primitive by cumulative area; `direct_light` samples a point uniformly on its surface, traces a visibility ray, and converts the area-domain estimate using distance and both surface cosines.

Environment loading stores linear RGB and a CDF proportional to texel luminance times `sin(theta)`. `sample_environment_importance` binary-searches the CDF, jitters inside the selected texel, and converts probability mass to a solid-angle PDF. `direct_environment` combines this explicit estimate with the diffuse BSDF using the power heuristic. When a BSDF-sampled diffuse ray reaches the environment, `ray_color` applies the complementary MIS weight using `environment_pdf`. Environment rotation affects evaluation, sampling, and PDF lookup consistently.

## Gradient shader

`gpu_gradient.wgsl` contains a smaller four-field `RenderParams`, bindings for output and parameters, and one `main`. Each in-bounds invocation writes `(u, v, 0.25, 1)`. It exercises adapter creation, buffers, bind groups, dispatch, synchronous readback, common image conversion, and PNG output without scene resources.

## Change checklist

When modifying a GPU record or binding:

1. Update the Rust and WGSL layouts together, including padding.
2. Update buffer creation, bind-group layout, and bind-group entries.
3. Confirm animation uploads do not exceed originally allocated buffer sizes.
4. Run `cargo test --workspace`, strict clippy, and `./scripts/check_docs.sh`.
5. Run a small GPU render and progressive preview; shader parsing happens only when a GPU pipeline is created.
