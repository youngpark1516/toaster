# Toaster scene format

Toaster scenes are UTF-8 JSON files loaded by `toaster_scene::load_scene`. All vectors are JSON arrays of three numbers. Asset paths are absolute or resolved relative to the directory containing the scene file.

## Top-level schema

```json
{
  "camera": { "position": [0, 1, 4], "look_at": [0, 0, 0], "fov_degrees": 45 },
  "render": { "width": 640, "height": 360, "samples": 16, "max_bounces": 8 },
  "physics": { "enabled": true, "type": "rigid_body" },
  "materials": [],
  "objects": [],
  "triggers": [],
  "spawn_points": [],
  "event_reactions": [],
  "animation": { "tracks": [] }
}
```

`camera`, `render`, `materials`, and `objects` are required. `physics` is
optional; `triggers`, `spawn_points`, `event_reactions`, and animation tracks default to empty arrays. Unknown
fields are currently ignored by some general scene structures;
physics-specific structures, trigger/spawn declarations, and event reactions reject
them.

## Camera

| Field | Type | Default | Rules |
| --- | --- | --- | --- |
| `position` | `[x,y,z]` | required | finite and different from `look_at` |
| `look_at` | `[x,y,z]` | required | finite |
| `up` | `[x,y,z]` | `[0,1,0]` | finite and not parallel to the view direction |
| `fov_degrees` | number | required | finite, strictly greater than 0 and less than 180 |

`vertical_fov_degrees` is accepted as a legacy alias for `fov_degrees`.

## Render settings

| Field | Type | Default | Rules |
| --- | --- | --- | --- |
| `width` | integer | required | greater than zero |
| `height` | integer | required | greater than zero |
| `samples` | integer | required | greater than zero |
| `max_bounces` | integer | required | stored as supplied; CLI overrides require greater than zero |
| `background` | string/object | `"sky"` | forms below |

`samples_per_pixel` is accepted as a legacy alias for `samples`. CLI render overrides are applied after loading and do not modify the scene file.

### Backgrounds

The compact values are:

```json
"background": "sky"
```

```json
"background": "black"
```

An HDR environment is the detailed tagged form:

```json
"background": {
  "type": "environment",
  "path": "../assets/environments/studio_test.hdr",
  "intensity": 1.0,
  "rotation_degrees": 0.0
}
```

`path` is required. `intensity` defaults to `1.0` and must be finite and nonnegative. `rotation_degrees` defaults to `0.0` and must be finite. Environment pixels are loaded as linear RGB, and Toaster builds a luminance-times-sine importance distribution for direct lighting. The environment is both visible on misses and sampled as illumination.

## Physics

The optional top-level block configures renderer-neutral scene evaluation:

```json
"physics": {
  "enabled": true,
  "type": "rigid_body",
  "gravity": [0.0, -9.81, 0.0],
  "timestep": 0.016666666666666666,
  "substeps": 1
}
```

| Field | Type | Default | Rules |
| --- | --- | --- | --- |
| `enabled` | boolean | `true` | disabled declarations are validated but not evaluated |
| `type` | string | required | MVP accepts only `rigid_body` |
| `gravity` | `[x,y,z]` | `[0,-9.81,0]` | finite, in meters per second squared |
| `timestep` | number | `1/60` | finite and positive, in seconds |
| `substeps` | integer | `1` | from 1 through 64 |

Object physics declarations require this top-level block. Rapier is the private
rigid-body implementation in `toaster-physics`; neither the scene format nor
renderer APIs expose Rapier types.

When `enabled` is `false`, all declarations are still parsed and validated, but
no backend is constructed and they impose no animation ownership restrictions.
The scene otherwise behaves like an ordinary scene, including under existing
progressive-preview eligibility rules.

## Materials

Materials have a unique `name` used by object references and a tagged `type`.

### Diffuse

```json
{ "name": "matte", "type": "diffuse", "albedo": [0.7, 0.2, 0.1] }
```

`albedo` channels must be finite and in `[0,1]`.

### Metal

```json
{ "name": "steel", "type": "metal", "albedo": [0.8, 0.8, 0.85], "roughness": 0.15 }
```

`albedo` follows the diffuse rules. `roughness` must be finite and is clamped to `[0,1]`.

### Dielectric

```json
{ "name": "glass", "type": "dielectric", "ior": 1.5 }
```

`ior` must be finite and greater than zero.

### Emissive

```json
{ "name": "light", "type": "emissive", "color": [1, 0.95, 0.8], "strength": 8 }
```

`color` channels must be finite and in `[0,1]`. `strength` must be finite and nonnegative. Positive-strength spheres and triangles enter the renderers' direct-light distributions.

`textured_diffuse` is a runtime-only material created from imported glTF base-color textures; it cannot be declared directly in scene JSON.

## Objects

Every object is tagged by `type`. Optional `group` values assign primitives to
animation targets; an explicit group must be nonempty. An optional object-level
`id` is a stable identity distinct from `group`. Every sphere or box with a
physics declaration requires a nonempty `id`; IDs must be unique across all
declared object IDs and triggers. Nonphysics objects may carry IDs but do not
participate in physics events in this version.

### Sphere

```json
{
  "id": "hero",
  "type": "sphere",
  "center": [0, 0, 0],
  "radius": 1,
  "material": "glass",
  "group": "hero"
}
```

The center must be finite, radius finite and positive, and material name known.

### Box

```json
{
  "id": "crate",
  "type": "box",
  "center": [0, 1, 0],
  "size": [1, 2, 1],
  "material": "matte",
  "group": "crate"
}
```

Centers and sizes must be finite, and every size component must be positive.
Boxes begin axis-aligned and are expanded deterministically into twelve ordinary
triangles. The generated triangles keep the box material and group.

### Rigid-body object declaration

Spheres, boxes, and imported `mesh` objects may contain rigid-body metadata.
Primitive spheres and boxes use the inferred `collider` form below:

```json
"physics": {
  "body": "dynamic",
  "collider": "cuboid",
  "mass": 2.0,
  "friction": 0.7,
  "restitution": 0.15,
  "initial_velocity": [0.0, 0.0, 0.0],
  "initial_angular_velocity": [0.5, 1.0, 0.25]
}
```

- `body` is required and is `static`, `dynamic`, or `kinematic`.
- `collider` is optional and inferred. If supplied, spheres require `sphere` and
  boxes require `cuboid`.
- Dynamic `mass` defaults to `1.0` and must be finite and positive. Static and
  kinematic bodies reject mass and both velocity fields.
- `friction` defaults to `0.5` and must be finite and nonnegative.
- `restitution` defaults to `0.0` and must be finite and between zero and one.
- Initial linear velocity is in meters per second and angular velocity in
  radians per second; all components must be finite.

Explicit triangle objects reject physics metadata. All three body kinds support
analytic sphere colliders and boxes support cuboids with half extents `size / 2`.
Kinematic bodies require a nonempty group. With physics enabled, that group must
be exclusive to one logical object and targeted by at least one animation track.
Their rendered triangle range is reconstructed from immutable base vertices at
the animation-driven Rapier pose.

### Trigger zones

The optional top-level `triggers` array declares invisible fixed sensors:

```json
"triggers": [
  {
    "id": "reset_box",
    "shape": "box",
    "center": [0.0, 0.5, -3.0],
    "size": [3.0, 1.0, 2.0]
  },
  {
    "id": "goal_sphere",
    "shape": "sphere",
    "center": [2.0, 1.0, -4.0],
    "radius": 1.25
  }
]
```

Triggers require a top-level physics block. Centers must be finite, sphere
radii finite and positive, and every box size component finite and positive.
Trigger IDs share the object-ID namespace. Triggers have no material, render
binding, mass, friction, restitution, group, or animation. They detect dynamic
and kinematic bodies, but ignore static bodies and other triggers. Sensors do
not affect contact response and never contribute primitives, BVH data, or
lights. When physics is disabled, trigger declarations are still validated but
no sensors or events are created.

### Triangle

```json
{
  "type": "triangle",
  "vertices": [[-1, 0, 0], [1, 0, 0], [0, 1, 0]],
  "material": "matte",
  "group": null
}
```

All vertices must be finite and non-collinear. Explicit JSON triangles currently carry no vertex normals or texture coordinates.

### glTF mesh

```json
{
  "type": "mesh",
  "path": "../assets/models/textured_quad.gltf",
  "group": "prop"
}
```

The loader traverses all glTF scene roots and child nodes, applies node world transforms, triangulates only indexed triangle-mode primitives, and imports positions plus optional normals and `TEXCOORD_0`. It imports base-color factors and the referenced base-color image as a runtime textured diffuse material. A primitive without a material receives white diffuse.

Use `"material": "matte"` to override all imported materials:

```json
{
  "type": "mesh",
  "path": "../assets/models/tetrahedron.gltf",
  "material": "matte"
}
```

An imported mesh can participate in rigid-body physics through one explicit
sphere or box proxy. The render mesh and collider remain separate:

```json
{
  "id": "visual_crate",
  "type": "mesh",
  "path": "../assets/models/physics_crate.gltf",
  "group": "visual_crate",
  "physics": {
    "body": "dynamic",
    "collider_proxy": {
      "shape": "box",
      "center": [-1.5, 3.0, -3.0],
      "size": [1.2, 1.2, 1.2]
    },
    "mass": 2.0
  }
}
```

Sphere proxies use `"shape": "sphere"`, a finite world-space `center`, and a
finite positive `radius`. Box proxies use a finite world-space `center` and
finite positive full `size` components. These values describe geometry after
glTF node transforms have been baked into imported vertices; bounds are never
fitted automatically. A physics mesh requires an ID and exactly one proxy,
rejects the primitive `collider` string, and moves every imported node and
primitive as one rigid triangle range around the proxy center. There is no
`"type": "gltf"` alias.

Static mesh proxies create fixed colliders without per-frame visual updates.
Dynamic proxies move and rotate the complete imported range. Kinematic mesh
proxies follow the same exclusive-group and required-track rules as primitive
kinematic bodies. Reset restores the imported authored pose and velocities;
teleport spawn positions refer to the proxy/body center while retaining every
vertex's authored offset.

Material flashes cover the complete bound mesh range. During a flash every
triangle uses the flash material; expiry, loop reset, rewind, reset, or teleport
restores each triangle's individual imported material. All authored materials
in a flashable mesh and the flash material must be non-emissive.

The override name must exist. Imported texture indices and triangle indices are validated. A triangle using a base-color texture must provide `TEXCOORD_0`. The current importer supports the image formats handled explicitly in `toaster-assets` and rejects unsupported encoded pixel layouts with context.

## Animation

Animation is optional:

```json
"animation": {
  "tracks": []
}
```

Tracks target either the camera or an object group:

```json
"target": { "type": "camera" }
```

```json
"target": { "type": "group", "name": "hero" }
```

Group targets must name at least one loaded sphere or triangle. Imported mesh triangles inherit the mesh object's group.

### Translation track

```json
{
  "type": "translation",
  "target": { "type": "group", "name": "hero" },
  "interpolation": "linear",
  "keyframes": [
    { "time": 0, "value": [0, 0, 0] },
    { "time": 2, "value": [2, 0, 0] }
  ]
}
```

### Rotation track

```json
{
  "type": "rotation",
  "target": { "type": "camera" },
  "axis": [0, 1, 0],
  "pivot": [0, 0, 0],
  "interpolation": "linear",
  "keyframes": [
    { "time": 0, "degrees": 0 },
    { "time": 8, "degrees": 360 }
  ]
}
```

`interpolation` is required and is `linear` or `step`. Every track needs at least one keyframe. Times must be finite, nonnegative, and strictly increasing. Translation values, rotation degrees, axes, and pivots must be finite. Rotation axes must be nonzero and are normalized during loading.

Evaluation always starts from the immutable base scene. All translation tracks are applied first (so their offsets compose), then rotation tracks in declaration order. Values clamp to the first/last keyframe outside their range. A camera translation moves both `position` and `look_at`; camera rotation orbits both about the pivot and rotates `up`. Group transforms update sphere centers and triangle positions; triangle normals rotate but translations do not affect them.

CLI animation time is `frame_index / fps`. For enabled physics, scene time is
wrapped by `--loop-duration` and converted to the latest completed fixed tick;
each nominal tick performs `substeps` steps of `timestep / substeps`. Before
every substep, kinematic tracks are sampled at that substep's endpoint and the
absolute target pose is sent to Rapier. This lets Rapier infer the velocity used
to affect dynamic contacts. There are no remainder steps or render-pose
interpolation. Dynamic and static frame-zero poses are authored; a kinematic
frame-zero pose is its animation sample at time zero. A loop wrap resets the
world before replaying from tick zero, while render RNG frame indices continue
increasing.

When physics is enabled, animation tracks cannot target a static or dynamic
physics group. Each kinematic body requires its own exclusive group and at least
one matching animation track; multiple translation and rotation tracks may
compose for that object. Camera animation and unrelated object animation remain
allowed. Dynamic bodies are controlled only by physics, static bodies stay
fixed, and kinematic bodies follow animation while participating as moving
colliders. When physics is disabled, required-track, exclusivity, and ownership
checks are skipped and normal animation behavior applies, although declaration
fields remain validated.

### Physics events

Each evaluated frame carries an ordered renderer-neutral physics event batch.
Physical body pairs emit `Started`, one `Stayed` event for each subsequent
nominal fixed tick during which the contact remains active, and `Exited`.
Triggers emit `Entered` and `Exited` only. Collision IDs are sorted
lexicographically so a public pair has one stable ordering. Events contain the
loop cycle, completed fixed tick, wrapped scene-local time, phase, and stable
entity IDs; contact points, normals, forces, and impulses are not exposed.

An evaluation that advances several ticks returns every crossed-tick event.
Evaluating the same tick again does not duplicate events. Loop changes,
backward seeks, and random access reset and replay the world and active-pair
state. Such a batch has `reset: true`; consumers clear prior event-driven state
before applying it. Reset does not synthesize exits for the discarded world.
Animation-only and disabled-physics evaluation return an empty batch.

### Event reactions

The optional top-level `event_reactions` array matches neutral physics events
and applies renderer-neutral material flashes, counter increments, or dynamic-body actions:

```json
"event_reactions": [
  {
    "match": {
      "type": "collision",
      "phase": "started",
      "object": "ball_red",
      "other": "room_floor"
    },
    "flash": {
      "target": "ball_red",
      "material": "impact_flash",
      "duration": 0.2
    },
    "counter": "floor_impacts"
  },
  {
    "match": {
      "type": "trigger",
      "phase": "entered",
      "trigger": "gate_zone",
      "object": "*"
    },
    "counter": "gate_entries"
  }
]
```

Every rule requires `match` and at least one of `flash`, `counter`, `reset`, or
`teleport`. A rule cannot contain both reset and teleport. Collision phases are
`started`, `stayed`, and `exited`; trigger phases are `entered` and `exited`.
Omitted filters and `"*"` are wildcards. Collision participants are unordered:
two wildcards match every pair, one concrete ID matches any pair containing it,
and two concrete IDs match that exact pair. Trigger and object filters are
independent.

A flash target must be a concrete rendered physics-object ID explicitly named
by its matcher. A wildcard cannot authorize a flash target. Trigger IDs and
nonphysics marker objects cannot be flashed. `duration` is finite, positive,
and measured in wrapped scene seconds. The active interval is half-open:
`[event_time, event_time + duration)`. A later event replaces the active flash;
ties follow declaration order. Expiry restores the immutable authored material.
Loop/reset replay first clears active flashes, preventing a late-cycle flash
from leaking into the next cycle. Flashes that begin and expire between output
frames are not held artificially.

The v4 MVP accepts only non-emissive authored and flash materials. It swaps an
existing material index, preserving all renderer buffer and light counts.

A counter increments once for each event matched by its rule, and multiple
rules may share a name. `stayed` counting is opt-in: a rule must explicitly use
`"phase": "stayed"`. Such a counter can increase quickly because an active
contact emits one stay event per nominal fixed tick.

Evaluated scenes expose `loop_counts` and `session_counts`. Loop counts clear
and replay on a wrap or rewind. Session counts continue across loop cycles and
deduplicate replay at or below the highest fixed tick already observed for that
cycle. Skipped cycles are not synthesized. With physics disabled, reactions
remain validated, flashes do nothing, and declared counter names remain zero.

### Reset and teleport actions

Reset zones use ordinary trigger events and a concrete dynamic-body action:

```json
{
  "match": {
    "type": "trigger",
    "phase": "entered",
    "trigger": "reset_zone",
    "object": "ball_0"
  },
  "reset": { "target": "ball_0" },
  "counter": "ball_resets"
}
```

Reset restores the stored authored initial pose and authored initial linear and
angular velocities, clears forces, torques, sleeping state, and any active
flash on the target. Current spheres and boxes have no authored object-rotation
field, so their authored initial rotation is identity; reset stores and uses a
neutral initial transform rather than hard-coding that limitation.

Teleports use a named top-level destination:

```json
"spawn_points": [
  {
    "name": "goal_spawn",
    "position": [2, 3, -1],
    "rotation": { "axis": [0, 1, 0], "degrees": 90 }
  }
]
```

```json
{
  "match": {
    "type": "trigger",
    "phase": "entered",
    "trigger": "goal_zone",
    "object": "crate_0"
  },
  "teleport": {
    "target": "crate_0",
    "spawn": "goal_spawn",
    "velocity": "clear"
  },
  "counter": "goal_teleports"
}
```

Spawn names are nonempty and unique. Positions are absolute finite body origins.
Rotation is optional, defaults to identity, and uses a finite nonzero axis plus
finite degrees. `velocity` defaults to `clear`, which zeros both linear and
angular velocity; `preserve` retains both. Teleport clears forces, torques,
sleep, and the target's active flash.

Reset and teleport targets must be concrete dynamic-body IDs explicitly named
by the matcher, using the same authorization rule as flashes. Wildcards cannot
authorize a motion target. Static and kinematic targets, inline destinations,
and named reset poses are not supported. Multiple matching actions execute in
event order and then declaration order, so the last action for a body wins.

Actions run after a nominal tick's event collection and before the next tick.
They synthesize no events in that tick; contacts and overlaps caused by the new
pose are observed normally on the following tick. Rewind and loop reset replay
all actions from tick zero. With physics disabled, spawn points and actions are
validated but actions do nothing and counters stay at zero.

Static progressive preview rejects every scene containing animation tracks and
every scene with enabled physics, including static-only physics scenes. Normal
MJPEG stream preview supports physics.

## Complete representative scene

```json
{
  "camera": {
    "position": [0, 1.2, 4],
    "look_at": [0, 0.5, 0],
    "up": [0, 1, 0],
    "fov_degrees": 45
  },
  "render": {
    "width": 640,
    "height": 360,
    "samples": 32,
    "max_bounces": 8,
    "background": {
      "type": "environment",
      "path": "../assets/environments/studio_test.hdr",
      "intensity": 1.0,
      "rotation_degrees": 15
    }
  },
  "materials": [
    { "name": "red", "type": "diffuse", "albedo": [0.75, 0.1, 0.08] },
    { "name": "lamp", "type": "emissive", "color": [1, 0.9, 0.7], "strength": 5 }
  ],
  "objects": [
    { "type": "sphere", "center": [0, 0.5, 0], "radius": 0.5, "material": "red", "group": "hero" },
    { "type": "sphere", "center": [0, 3, 0], "radius": 0.4, "material": "lamp" },
    { "type": "mesh", "path": "../assets/models/tetrahedron.gltf", "material": "red" }
  ],
  "animation": {
    "tracks": [
      {
        "type": "rotation",
        "target": { "type": "group", "name": "hero" },
        "axis": [0, 1, 0],
        "pivot": [0, 0.5, 0],
        "interpolation": "linear",
        "keyframes": [
          { "time": 0, "degrees": 0 },
          { "time": 10, "degrees": 360 }
        ]
      }
    ]
  }
}
```

For known-good files, see [`scenes/`](../scenes/). The root
[README](../README.md) contains the primary runtime commands.
