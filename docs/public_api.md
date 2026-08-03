# Public API reference

This page is a concise lookup for current exported Rust APIs and process-level interfaces. See [codebase_reference.md](codebase_reference.md) for internal functions and runtime ownership, and generated Rustdoc for signatures with source navigation.

## `toaster-core`

### Types

- `Camera`: validated pinhole camera. `Camera::new(position, look_at, up, vertical_fov_degrees, aspect_ratio)` rejects non-finite/degenerate vectors, invalid FOV, and nonpositive aspect ratios. `ray(u, v)` creates a primary ray.
- `ImageBuffer`: dense row-major linear-RGB pixels. `new`, `width`, `height`, `set_pixel`, `pixel`, and `save_png` provide construction, indexed access, and encoded output. Pixel access panics when coordinates are outside the declared dimensions.
- `Ray`: world-space `origin` and `direction`; `new` constructs it and `at(distance)` evaluates a point.

### Functions

- `linear_to_rgb8(Vec3) -> [u8; 3]`: clamps, gamma-corrects, and quantizes a linear color.

These types live in the public `camera`, `image_buffer`, and `ray` modules.

## `toaster-assets`

### Types

- `Mesh`: imported vertices, indexed triangles, materials, and textures. `triangle_vertices()` resolves each indexed triangle.
- `MeshVertex`, `MeshTriangle`, `MeshMaterial`, and `MeshTexture`: renderer-neutral glTF import records.

### Functions

- `load_gltf(path) -> Result<Mesh>`: imports glTF/GLB scene nodes, transforms positions/normals, reads `TEXCOORD_0`, base-color factors/textures, and validates indices.

The crate root re-exports the mesh types and `load_gltf`.

## `toaster-scene`

### Runtime model

- `Scene`: camera, render settings, materials, spheres, triangles, parallel triangle attributes, textures, optional environment, and animation. `evaluate_at(seconds)` clones the base scene and applies all tracks.
- `CameraSettings`, `RenderSettings`, and `Background`: renderer-neutral input settings.
- `Material`: `Diffuse`, imported-only `TexturedDiffuse`, `Metal`, `Dielectric`, or `Emissive`.
- `Sphere`, `Triangle`, and `TriangleAttributes`: runtime primitives and optional imported shading attributes.
- `Texture`: validated packed RGBA8 data. `new` validates dimensions/length; `sample_linear` performs repeating bilinear sampling with sRGB decoding.
- `EnvironmentMap` and `EnvironmentSample`: linear HDR equirectangular data and importance distribution. `new`, `load`, `sample`, `sample_importance`, `pdf_solid_angle`, and `importance_entries` expose evaluation and sampling.
- `Animation`, `AnimationTrack`, `AnimationTarget`, `Interpolation`, `TranslationKeyframe`, and `RotationKeyframe`: deserialized animation model. `AnimationTrack::target` returns its target.

### Functions

- `load_scene(path) -> Result<Scene>`: parses and validates JSON, resolves relative assets, expands meshes, loads environments, and validates animation.

The crate root re-exports all of the types and `load_scene`. The JSON contract is documented in [scene_format.md](scene_format.md).

## `toaster-cpu`

### Types

- `HitRecord`: distance, point, oriented/geometric normals, face orientation, material index, and texture coordinates.
- `AreaLights` and `LightSample`: collected emissive triangles and sampled point/normal/emission/PDF data. `collect`, `len`, `is_empty`, and `sample` manage the distribution.

### Functions

- `render(&Scene) -> ImageBuffer`: complete CPU image render.
- `ray_color(ray, scene, rng) -> Vec3`: traces one stochastic path.
- `intersect_sphere`, `intersect_triangle`, and `intersect_scene`: brute-force intersection queries.

## `toaster-gpu`

### Scheduling and sinks

- `AnimationConfig`: `single_frame`, `from_duration`, `from_frame_count`, and `indefinite`; optional `with_loop_duration`; accessors `frame_limit`, `time_for_frame`, `fps`, and `loop_duration`.
- `FramePacing`: `Unpaced` or `RealTime`.
- `FrameSink`: synchronous `deliver(CompletedFrame)` plus cancellation (`should_continue`) and optional next-sample override (`samples_for_frame`).
- `CompletedFrame`: frame/time/dimensions, batch and accumulated samples, bounce count, detailed timings, compatible total `render_time`, and a borrowed shared RGBA image.
- `ProgressiveRenderConfig`: nonzero target constructed with `new`; `target_samples` accessor.

### Rendering

- `render_scene_gpu`: one PNG frame.
- `render_scene_gpu_animation`: finite PNG sequence.
- `render_scene_gpu_video`: finite H.264 MP4 through FFmpeg.
- `render_scene_gpu_animation_with_sink`: load then run a sink-driven schedule.
- `render_gpu_animation_with_sink`: sink-driven rendering of an already loaded scene.
- `render_gpu_progressive_with_sink`: static linear-HDR accumulation to an exact target, then preview hold.
- `render_gradient`: diagnostic GPU compute/readback PNG.

### Benchmarking

- `GpuBenchmarkConfig`: validated warmup/measured frame counts with accessors.
- `GpuBenchmarkResult`: adapter metadata, setup duration, measured `GpuFrameTimings`, and the final RGBA image.
- `GpuAdapterInfo`: adapter/backend/device/driver strings.
- `benchmark_gpu_scene`: time-zero, fixed-seed, independent frames using one GPU setup.

### Lower-level GPU interfaces

Public modules expose C-compatible `gpu_types`, `buffers`, device creation, dispatch, image conversion/encoding, pipeline construction, readback, and scene upload. These are maintained interfaces but normally callers should prefer the high-level render functions. `SceneGpuData` is the packed host representation. See generated Rustdoc and [shader_reference.md](shader_reference.md) for host/shader layout correspondence.

## `toaster-server`

### Types

- `FramePublisher`: cloneable latest-value broadcaster. `new`, `publish_jpeg`, and `publish_frame` replace the current JPEG/status without queueing old frames.
- `PreviewStatus`: serializable frame, pacing, sampling, progressive-completion, and bounce metadata.

### Functions

- `bind(host, port)`: explicitly claims a TCP address before renderer setup.
- `serve_listener(listener, publisher)`: serves an already-bound listener.
- `serve_with_publisher(host, port, publisher)`: bind and serve with an existing publisher.
- `serve(host, port)`: standalone server with a fresh publisher.

## `toaster-bvh`

The public `aabb`, `bvh`, and `flatten` modules are currently empty integration contracts. No BVH symbols or traversal behavior are implemented on this branch. The incoming BVH work is expected to own finite primitive bounds, hierarchy construction, a flat traversal representation, and CPU/GPU integration data without moving scene ownership into this crate.

## CLI interface

Global options are `--log-level error|warn|info|debug|trace` and `--log-format text|json`. Explicit level overrides `RUST_LOG`; logs go to stderr.

| Command | Purpose | Required output/behavior |
| --- | --- | --- |
| `cpu-render SCENE --out PNG` | CPU still render | accepts width/height/samples/bounce overrides |
| `gpu-render SCENE --out PNG` | GPU still or numbered animation | animation requires `--fps` and exactly one of `--duration`/`--frames` |
| `gpu-render SCENE --video MP4` | GPU H.264 export | requires FFmpeg, FPS, and a finite schedule |
| `stream-preview SCENE` | Live GPU MJPEG | defaults to `127.0.0.1:7878`, 12 FPS; optional duration, loop, adaptive, or static progressive controls |
| `benchmark SCENE --out JSON` | Repeatable GPU benchmark | optional comparison, threshold, and reference PNG |
| `server` | Empty standalone latest-frame HTTP service | listener host/port configurable |
| `info` | Version and module summary | writes human command output to stdout |

Run `toaster COMMAND --help` for the authoritative flag grammar.

## HTTP interface

| Route | Response |
| --- | --- |
| `GET /` | Minimal preview HTML with `<img src="/stream">` and a status panel |
| `GET /stream` | `multipart/x-mixed-replace; boundary=frame` MJPEG stream; complete JPEG parts, no cache |
| `GET /status` | JSON `PreviewStatus`, or `null` before the first published frame |
| `GET /healthz` | HTTP 200 with `ok` |

Every client observes the same watch-channel latest value. Slow clients skip superseded frames and never backpressure rendering.
