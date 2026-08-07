# Architecture

Toaster loads JSON, glTF/GLB geometry, textures, environment maps, and optional
physics declarations into a renderer-independent scene. The CPU integrator
provides a readable reference path. The `wgpu` integrator uploads the evaluated scene and flattened BVH,
dispatches WGSL compute work, and returns the result through one shared readback
and RGBA conversion path. The root [README](../README.md) diagrams this flow.

The workspace keeps scene and asset code independent from renderer details:
`toaster-core` and `toaster-scene` hold shared data, `toaster-physics` evaluates
renderer-neutral simulation updates, `toaster-cpu` and `toaster-gpu` implement
the renderers, `toaster-bvh` builds acceleration data, and the CLI/server crates
own presentation and transport.

## Engineering decisions

### Physics is a frame-evaluation backend

The frame pipeline is:

```text
immutable base scene
  -> allowed animation evaluation
  -> backend-neutral physics evaluation
  -> evaluated renderer-neutral scene
  -> GPU packing/BVH/upload
  -> render/output
```

`toaster-scene` owns JSON declarations, stable object bindings, rigid transforms,
and the general `SceneEvaluator` interface. `toaster-physics` owns the
`PhysicsBackend` abstraction and the first implementation,
`RapierRigidBodyBackend`. Rapier handles are private implementation details: no
Rapier type crosses into scene, CPU renderer, GPU renderer, or WGSL APIs. This
boundary leaves room for future research backends without pretending that a
PDE or neural solver should replace rigid-body contact physics.

Every requested frame is evaluated from a clone of the immutable source scene.
Physics outputs absolute renderer-neutral transforms tied to stable sphere or
triangle-range bindings. Monotonic playback reuses a stepped world; loop wraps,
backward seeks, and random access reset and replay it. Results are intended to
be deterministic within the same platform, build, dependency versions,
configuration, and scene construction order, not universally bit-identical
across machines.

Enabled rigid-body groups have exclusive ownership: neither dynamic nor static
physics groups may be animation targets. Camera and unrelated object animation
remain valid. When the top-level physics block is disabled, declarations are
still validated but do not reserve groups and no backend is constructed.

Dynamic boxes are authored as first-class boxes and expanded into twelve normal
triangles. A full mixed-BVH rebuild is uploaded whenever sphere or triangle
geometry moves. Moving emissive geometry also rebuilds the light list. This
prioritizes correctness; BVH refitting is a future optimization.

The current backend is intentionally limited to rigid bodies: gravity, contact,
friction, restitution, and linear/angular motion for static and dynamic spheres
and cuboids. Possible later research backends include `PinnHeatBackend`,
`PinnWaveBackend`, `PinnFluidBackend`, `NeuralSurrogateBackend`, and
`DifferentiablePhysicsBackend` for PDE-like fields, inverse simulation, or
learned models. No PINN schema or solver is part of this milestone. Joints,
constraints, fluids, cloth, soft bodies, fracture, GPU physics, glTF rigid
bodies, and interactive controls are also outside its scope.

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
image. Enabled physics is rejected for the same reason, including scenes whose
physics bodies are all static. Independent frames use a prior count of zero.

### Portable compute with one output boundary

Toaster uses `wgpu` rather than CUDA to support multiple GPU vendors and native
backends. This fits cluster experiments and future browser-facing work, at the
cost of CUDA-specific tooling and vendor libraries.

Every GPU output follows the same boundary: linear floating-point storage,
readback, then one RGBA8 conversion. PNG, MJPEG, video, and benchmark paths
therefore agree on pixel conversion. FFmpeg consumes raw RGBA frames over stdin,
avoiding temporary PNGs; this is not direct GPU-memory encoding.
