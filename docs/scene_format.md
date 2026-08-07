# Toaster scene format

Toaster scenes are UTF-8 JSON files loaded by `toaster_scene::load_scene`. All vectors are JSON arrays of three numbers. Asset paths are absolute or resolved relative to the directory containing the scene file.

## Top-level schema

```json
{
  "camera": { "position": [0, 1, 4], "look_at": [0, 0, 0], "fov_degrees": 45 },
  "render": { "width": 640, "height": 360, "samples": 16, "max_bounces": 8 },
  "materials": [],
  "objects": [],
  "animation": { "tracks": [] }
}
```

`camera`, `render`, `materials`, and `objects` are required. `animation` defaults to an empty track list. Unknown fields are currently ignored by Serde; do not rely on that for forward compatibility.

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

Every object is tagged by `type`. Optional `group` values assign primitives to animation targets; an explicit group must be nonempty.

### Sphere

```json
{
  "type": "sphere",
  "center": [0, 0, 0],
  "radius": 1,
  "material": "glass",
  "group": "hero"
}
```

The center must be finite, radius finite and positive, and material name known.

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

CLI animation time is `frame_index / fps`. `--loop-duration` wraps that time while indices continue increasing. Static progressive preview rejects every scene containing animation tracks so samples from different poses cannot mix.

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
