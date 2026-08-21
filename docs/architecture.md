# Architecture

Toaster loads JSON, glTF/GLB geometry, textures, environment maps, and optional
physics declarations into a renderer-independent scene. The CPU integrator
provides a readable reference path. The `wgpu` integrator uploads the evaluated
scene and flattened BVH, dispatches WGSL compute work, and returns the result
through one shared readback and RGBA conversion path. The root
[README](../README.md) diagrams this flow.

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
  -> fixed-tick physics + neutral event/action feedback
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

Physics objects and invisible triggers use validated scene-authored
`PhysicsEntityId` values. These identities are independent of render bindings
and animation groups: a binding locates flattened geometry, a group defines
shared animation ownership, and an entity ID identifies one public event
participant. Rapier collider handles and event flags remain private to
`toaster-physics`.

Rapier contact transitions are converted immediately after every substep into
renderer-neutral collision and trigger events. Physical contacts expose
`Started`, one synthesized `Stayed` event per persistent nominal tick, and
`Exited`; sensor overlaps expose `Entered` and `Exited`. Collision object IDs
are canonically ordered. One evaluation batch contains every event from all
fixed ticks crossed by that request, while evaluating the same tick twice emits
no duplicate events. Event-only activity is separate from `SceneChanges` and
therefore does not cause geometry, BVH, light, or GPU uploads.

Scene-authored event reactions consume that neutral stream after physics
ticks. The response layer can temporarily swap a concrete physics body's
material index, increment named counters, and issue neutral dynamic-body state
commands for authored reset or named-spawn teleport. It has no Rapier or
renderer types: GPU and CPU paths see only an ordinary evaluated scene plus a
neutral counter snapshot. Non-emissive material swaps invalidate the bound
sphere or triangle data without changing material, light, primitive, or BVH
counts, so WGSL and GPU buffer layouts remain unchanged.

Loop counters are reconstructed from replayed events. Session counters retain
telemetry across loops while a per-loop high-water tick prevents rewinds from
double-counting already observed ticks. Session values therefore describe the
request history of one evaluator and never affect rendered state. Counter
snapshots travel through completed GPU frames to the existing preview `/status`
response; image, video, and benchmark sinks may ignore them.

Loop changes and backward seeks reset the simulation and its active-pair sets,
then replay from tick zero. The returned event batch marks that reset so a
consumer can clear its own event-driven state before applying replayed events;
discarded contacts do not receive synthetic exits. Renderers are allowed to
ignore event batches.

The same reset clears active material flashes before replay. A flash near a
loop boundary cannot leak into the next cycle; it reappears only if its matching
event occurs again in that cycle.

Scenes with motion actions are stepped one nominal tick at a time. Events from
each tick are matched before the next tick advances, and resulting reset or
teleport commands carry only stable IDs, neutral poses, and velocity policy
through `PhysicsBackend`. Rapier privately resolves handles, changes the body
state, clears forces/torques, and wakes it. No secondary event is synthesized
during action application; the next tick detects any resulting contact or
sensor transition. This ordering also makes incremental playback equivalent to
reset-and-replay when one rendered frame crosses many fixed ticks.

Enabled rigid-body groups have explicit ownership. Dynamic bodies are driven by
Rapier and static bodies stay fixed, so neither kind may be an animation target.
Kinematic bodies instead require one exclusive animation group and matching
tracks. Rapier samples that neutral group pose at every physics substep and uses
a private position-based kinematic body, allowing it to push dynamic bodies
without being displaced by gravity or contacts. Camera and unrelated object
animation remain valid. When the top-level physics block is disabled,
declarations are still validated but ownership checks are skipped and no
backend is constructed.

Dynamic boxes are authored as first-class boxes and expanded into twelve normal
triangles. Imported `mesh` objects can now bind one explicit renderer-neutral
sphere or cuboid proxy to their entire contiguous triangle range. glTF node
transforms remain baked into authored vertices; the proxy center becomes the
rigid-body origin, while Rapier sees only the lowered simple collider. Static,
dynamic, and exclusive-group kinematic mesh bodies therefore reuse the same
transform, event, reaction, reset, and teleport paths as primitive bodies.
Multi-material flashes are represented as a temporary range override, so a
fresh immutable-scene clone restores every triangle's own authored material.

A full mixed-BVH rebuild is uploaded whenever sphere or triangle
geometry moves. Moving emissive geometry also rebuilds and uploads the light
list; non-emissive motion reuses the prior light list. This prioritizes
correctness; BVH refitting is a future optimization.

Triggers are invisible fixed Rapier sensors with neutral sphere or cuboid
geometry. They detect dynamic and kinematic bodies without changing motion and
never add render primitives, BVH nodes, or lights. Static bodies and other
triggers are not trigger-event participants.

Frame diagnostics separately record animation evaluation, physics stepping,
neutral geometry updates, light-list and BVH rebuilds, GPU uploads, dispatch,
readback, RGBA conversion, output delivery, and total time. These are host
wall-clock measurements intended to expose the full-rebuild cost, not a GPU
hardware-profiler replacement.

The current backend is intentionally limited to rigid bodies: gravity, contact,
friction, restitution, dynamic linear/angular motion, and animation-driven
kinematic motion for spheres and cuboids. Possible later research backends include `PinnHeatBackend`,
`PinnWaveBackend`, `PinnFluidBackend`, `NeuralSurrogateBackend`, and
`DifferentiablePhysicsBackend` for PDE-like fields, inverse simulation, or
learned models. No PINN schema or solver is part of this milestone. Joints,
constraints, fluids, cloth, soft bodies, fracture, GPU physics, dynamic
triangle-mesh colliders, convex decomposition, skeletal animation, and
interactive controls are also outside its scope.

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

Every renderer output follows the same boundary: linear floating-point storage,
then one CPU-side display conversion. Exposure scales radiance by
`2^exposure_stops`; the ACES fitted curve compresses HDR highlights; standard
linear-to-sRGB encoding produces display bytes. GPU output is converted after
readback. CPU PNG, GPU PNG, MJPEG, video, and benchmark paths therefore agree on
pixel conversion. FFmpeg consumes the resulting raw RGBA frames over stdin,
avoiding temporary PNGs; this is not direct GPU-memory encoding.
