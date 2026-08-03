# Toaster codebase reference

This is the implementation map for the current working tree. It describes ownership, runtime flow, and every non-test production Rust function/method. Tests are documented by coverage rather than cataloged individually. Generated private-item Rustdoc remains the source-linked companion to this guide.

## Repository map

```text
toaster/
├── Cargo.toml                    workspace dependencies and documentation lints
├── crates/
│   ├── toaster-cli/              process entry point and orchestration
│   ├── toaster-core/             camera, ray, color, CPU image buffer
│   ├── toaster-assets/           glTF/GLB import and mesh intermediate data
│   ├── toaster-scene/            JSON loading, validation, animation, environments
│   ├── toaster-cpu/              reference path tracer and intersections
│   ├── toaster-gpu/              wgpu path tracer, sinks, video, benchmarks
│   ├── toaster-server/           Axum latest-frame MJPEG/status service
│   └── toaster-bvh/              in-progress, currently empty BVH contract
├── shaders/                      active and placeholder WGSL
├── scenes/                       checked-in examples and smoke-test inputs
├── assets/                       glTF, textures, and environment inputs
├── docs/                         user, architecture, operation, and API guides
├── scripts/                      repeatable validation/smoke-test entry points
├── out/                          ignored generated images/video/reports/logs
└── target/                       ignored Cargo output and generated Rustdoc
```

Important ownership rule: `toaster-scene` owns renderer-neutral validated state. Asset import may add data to that state, but neither renderer owns scene parsing. `toaster-cpu` and `toaster-gpu` consume the same scene. `toaster-server` knows only encoded JPEG bytes and status. The CLI is the composition root.

## Crate dependency graph

```text
toaster-cli ──► toaster-cpu ──► toaster-core
     │                └───────► toaster-scene ──► toaster-assets
     ├────────► toaster-gpu ──► toaster-core     └──────────────┐
     │                └───────► toaster-scene                   │
     └────────► toaster-server                                  │
                                                               │
toaster-bvh (currently standalone contract; integration pending)◄┘
```

External responsibilities are intentionally narrow: `glam` math, `image` image/HDR decoding and output, `gltf` import, `rand` CPU sampling, `wgpu` compute, `axum`/Tokio HTTP runtime, `tracing` diagnostics, and Clap CLI parsing. FFmpeg is invoked as an external process only for MP4 output.

## End-to-end flows

### Scene loading

`toaster_scene::load_scene` reads JSON relative to its own directory, deserializes private input records, validates camera/render/material/object data, calls `toaster_assets::load_gltf` for meshes, rebases imported material/texture indices, loads an optional HDR environment, normalizes rotation axes, and validates animation targets/keyframes. The result is a single immutable base `Scene`. Animation evaluation clones this base, applies all translations, then rotations.

### CPU render

The CLI loads the scene, applies render overrides, and calls `toaster_cpu::render`. A validated `Camera` produces jittered primary rays. `ray_color` brute-force intersects scene primitives, adds explicit area/environment lighting for diffuse hits, scatters through materials, and stops at miss/emission/depth. Linear pixels enter `ImageBuffer`; `save_png` performs display conversion and filesystem output. CPU rendering is synchronous and currently single-process code without a Tokio runtime.

### GPU still and animation

The CLI resolves an `AnimationConfig` and calls the PNG wrapper. GPU code loads/evaluates the scene, packs `SceneGpuData`, creates one adapter/device/queue, allocates buffers, compiles WGSL, and creates one pipeline. Each frame evaluates animation at `frame_index / fps`, uploads mutable camera/geometry/params, dispatches, synchronously waits, reads floating-point pixels, creates one shared RGBA image, then gives the borrowed image to a sink. The PNG sink writes the base path for one frame or numbered sibling paths for an animation.

### Video export

The same single-setup GPU loop feeds `VideoFrameSink`. `FfmpegVideoWriter` starts FFmpeg configured for raw packed RGBA input and H.264/yuv420p MP4 output. Each completed image is written in full to the child pipe; measured GPU frame time excludes that write. Completion closes stdin and validates FFmpeg's exit status. Drop kills unfinished children as a cleanup fallback.

### Live preview

The CLI binds the TCP listener before GPU setup, creates a latest-frame `FramePublisher`, spawns the Axum server and Ctrl+C task, then runs the GPU future in the Tokio runtime. `ServerFrameSink` converts the same RGBA image to quality-90 JPEG, replaces the watch-channel value/status, and consults an atomic cancellation flag between frames. Real-time pacing sleeps to absolute `frame_index / fps` deadlines only when ahead; slow rendering skips sleep and creates no queue. Shutdown sets cancellation, aborts/awaits background tasks, and propagates render/server failures.

### Progressive preview

Progressive mode rejects animation tracks and loop duration, evaluates time zero only, and treats each dispatch sample count as a batch. The shader combines the batch average with binding 0's prior linear-HDR average using `accumulated_samples`. Rust clamps the final batch to the exact target, publishes every intermediate RGBA/JPEG/status update, then performs no more GPU work. The server holds the converged latest frame until the finite preview deadline or cancellation. Adaptive mode can alter the next batch within min/max/remaining bounds.

### Benchmark

The CLI loads and overrides a scene, then `benchmark_gpu_scene` uses the shared GPU loop in static-independent mode: time zero, frame seed zero, one setup, discarded warmups, and measured frames. Stage timings cover scene update/upload, dispatch plus wait, readback/map, conversion, and total. JSON/PNG writes are outside measurements. The CLI report layer records schema/version/time, resolved counts/settings, adapter/driver identity, all samples, statistics, and a SHA-256 of final RGBA bytes. Comparison requires matching schema/settings; thresholds require matching GPU identity.

### glTF and environment data

glTF import walks default/all scenes recursively, composes node transforms, emits indexed triangles with transformed positions and inverse-transpose normals, and carries optional UVs/materials/textures. Scene loading either applies a Toaster material override or converts imported base-color data to diffuse/textured diffuse runtime materials. Environment loading decodes an image to linear RGB and constructs a normalized luminance-times-sine CDF. CPU and GPU renderers both evaluate and importance-sample that representation.

## Cross-cutting behavior

### Logging and output

`tracing` logs go to stderr in text (`info` default) or JSON. Explicit `--log-level` overrides `RUST_LOG`. Lifecycle/configuration/output summaries are `info`; setup stages, per-frame timings, publications, and HTTP requests are `debug`; recoverable differences and signal/server/encoding issues use `warn`/`error`. The `info` subcommand alone prints its intended human output to stdout. PNG, MP4, benchmark JSON/reference PNG, and MJPEG are the only material outputs.

### Runtime and threading

CPU, ordinary GPU, and benchmark commands are synchronously driven (GPU async setup is bridged with `pollster`). Preview creates a Tokio runtime. Axum and Ctrl+C are Tokio tasks; the sink-driven renderer runs within that runtime but wgpu waits and real-time sleeps are synchronous. The watch channel is latest-value state, not a work queue. FFmpeg is a child process with one stdin producer.

### GPU lifetime and invariants

One render session owns one `GpuContext`, pipeline, bind group, output/readback pair, and scene buffers. Static texture/environment/material buffers do not change. Camera, parameters, spheres, triangles, attributes, and lights may be rewritten without reallocation, so animation cannot change primitive counts. Host and WGSL layouts/bindings must remain identical. Readback happens only after a synchronous queue completion.

### Cancellation and errors

`FrameSink::should_continue` is checked between frames and before pacing. No in-flight dispatch is interrupted. Most APIs return `anyhow::Result` with context for I/O, validation, adapter/device, mapping, encoding, or child-process failures. Indexed image access and internal post-validation assumptions use panics/`expect`; external scene/CLI input should be rejected earlier.

## Modules and production function catalog

Visibility is `public`, `crate`, or `private`. Unless stated otherwise, functions are deterministic for their explicit inputs, make no external writes, and are called by the owning module's higher-level workflow. “Errors” includes returned `Result` failures; “invariant” notes preconditions that are validated or assumed.

### `toaster-core`

`camera.rs` owns validated pinhole geometry.

| Function | Visibility and contract |
| --- | --- |
| `Camera::new` | Public. Inputs look-at vectors, vertical FOV, and aspect; returns validated camera or error for non-finite/degenerate geometry. Allocates nothing externally. Called by CPU render/tests. |
| `Camera::ray` | Public. Inputs normalized viewport coordinates; returns a world ray. Assumes a camera created by `new`. Called per CPU sample. |

`color.rs` owns display conversion.

| `linear_to_rgb8` | Public. Inputs linear RGB; returns clamped square-root-gamma RGB8. No errors/side effects. Called by CPU PNG encoding. |

`image_buffer.rs` owns CPU output memory.

| Function | Visibility and contract |
| --- | --- |
| `ImageBuffer::new` | Public. Allocates `width*height` black pixels; integer conversion/allocation must fit memory. Called by CPU render. |
| `width`, `height` | Public accessors returning declared dimensions. |
| `set_pixel` | Public. Replaces one row-major pixel; panics out of bounds. Called by CPU render. |
| `pixel` | Public. Copies one pixel; panics out of bounds. Called by output/tests. |
| `save_png` | Public. Converts every linear pixel and writes PNG; returns filesystem/encoder errors. Called by CLI CPU command. |

`ray.rs` owns geometric rays.

| `Ray::new` | Public constructor; stores origin/direction without normalization. Callers own direction invariants. |
| `Ray::at` | Public evaluator returning `origin + distance*direction`. Used by intersection code. |

### `toaster-assets`

`mesh.rs` defines the intermediate glTF records.

| `Mesh::triangle_vertices` | Public. Lazily resolves each triangle's three vertex indices; panics only if an importer constructed an invalid `Mesh`. Called by scene expansion. |

`gltf_loader.rs` owns import.

| Function | Visibility and contract |
| --- | --- |
| `load_gltf` | Public. Reads glTF/GLB and buffers/images, walks scene roots, validates triangle/index/attribute data, and returns `Mesh`; filesystem/parse/unsupported-data failures carry path context. Called by scene mesh loading. |
| `load_scene_nodes` | Private. Iterates root nodes with identity parent transform and appends geometry. Mutates output vectors. Called by `load_gltf`. |
| `load_node` | Private recursive walker. Composes world transforms, transforms positions/normals, imports primitive indices/material references, then children; rejects missing positions/indices and non-triangle modes. |
| `image_to_rgba8` | Private. Converts supported glTF decoded image formats to packed RGBA8 or errors. Called while building imported texture records. |

### `toaster-scene`

`loader.rs` owns JSON parsing, defaults, validation, and asset expansion.

| Function | Visibility and contract |
| --- | --- |
| `BackgroundFile::default` | Private trait method returning procedural sky when `background` is absent. Invoked by Serde. |
| `default_up` | Private Serde default returning positive Y. |
| `default_environment_intensity` | Private Serde default returning `1.0`. |
| `load_scene` | Public. Reads/parses a path and delegates to `build_scene`; returns contextual I/O/JSON/validation/asset errors. Principal caller: CLI and GPU path wrappers. |
| `build_scene` | Private. Validates raw fields, resolves names/paths, expands glTF, loads environment, normalizes/validates animation; returns the renderer-neutral `Scene`. Side effects are asset file reads. |
| `collect_options` | Private. Converts three optional attributes to `Some([T;3])` only when all are present. Called by mesh expansion. |
| `validate_group` | Private. Preserves `None`, rejects empty names. |
| `validate_triangle` | Private. Rejects non-finite or collinear vertices. |
| `validate_albedo` | Private. Delegates bounded RGB validation for a named material. |
| `validate_color` | Private. Rejects non-finite channels or values outside `[0,1]`; produces field-specific errors. |

`animation.rs` owns scene evaluation.

| Function | Visibility and contract |
| --- | --- |
| `AnimationTrack::target` | Public accessor returning the borrowed group/camera target. |
| `Animation::normalize_rotation_axes` | Crate. Validates and normalizes each rotation axis in place; called during loading. |
| `Scene::evaluate_at` | Public. Validates a finite nonnegative time, clones the base scene, applies translation then ordered rotation tracks; errors on invalid animation. Called per rendered animation frame. |
| `validate_animation` | Crate. Checks triangle-attribute alignment, group targets, axes/pivots, keyframe time/value validity. Called on load and evaluation. |
| `validate_times` | Private. Requires a nonempty, finite, nonnegative, strictly increasing sequence. |
| `sample_translation` | Private. Interpolates/clamps translation keyframes. |
| `sample_rotation` | Private. Interpolates/clamps degree keyframes. |
| `sample_segment` | Private generic adjacent-key lookup returning endpoints and normalized blend. Assumes validated sorted keys. |
| `endpoint_translation` | Private. Returns the first/last translation outside the key range. |
| `endpoint_rotation` | Private. Returns the first/last angle outside the key range. |
| `apply_translation` | Private. Mutates camera position/look-at or grouped primitive positions. |
| `apply_rotation` | Private. Mutates camera position/look-at/up or grouped geometry/normals around a pivot. |

`texture.rs` owns imported base-color sampling.

| `Texture::new` | Public. Validates nonzero dimensions and exact `width*height*4` bytes; returns a texture or error. |
| `Texture::sample_linear` | Public. Repeats UVs, bilinearly samples four texels, and returns linear RGB. Called by CPU shading. |
| `Texture::texel_linear` | Private. Wraps signed coordinates and decodes one RGBA8 texel. |
| `srgb_to_linear` | Crate. Piecewise-decodes one byte channel. Used by texture tests/sampling. |

`environment.rs` owns HDR evaluation and importance sampling.

| Function | Visibility and contract |
| --- | --- |
| `EnvironmentMap::new` | Public. Validates dimensions/pixel count/intensity/rotation, builds distribution, returns map or error. |
| `EnvironmentMap::load` | Public. Decodes an image as RGB32F then calls `new`; performs filesystem reads. |
| `sample` | Public. Bilinearly evaluates rotated equirectangular radiance; zero direction yields black. |
| `sample_importance` | Public. Inputs two unit-interval variates; inverts CDF and returns direction/radiance/PDF, or `None` for zero importance. |
| `pdf_solid_angle` | Public. Returns the sampling density for a direction or zero at invalid/polar/zero-probability cases. |
| `importance_entries` | Public. Iterates the shader payload `[weight,cdf]` per texel. |
| `direction_to_uv` | Private. Normalizes direction and applies yaw; returns `(u,v,theta)` or `None`. |
| `texel_pdf_solid_angle` | Private. Converts texel mass to solid-angle density. |
| `texel` | Private. Wraps X/clamps Y and returns one linear texel. |
| `build_importance_distribution` | Private. Computes luminance-times-sine weights and normalized CDF, including uniform fallback behavior for zero total. |
| `unit_interval` | Private. Clamps a sample to the representable half-open unit interval. |

`material.rs`, `object.rs`, and `scene.rs` are data-only production modules. `transform.rs` is a documented reserved boundary with no symbols; animation currently performs transforms directly.

### `toaster-cpu`

`integrator.rs` owns path sampling.

| Function | Visibility and contract |
| --- | --- |
| `render` | Public. Allocates the output, camera, light distribution, and RNG work; traces `samples` paths per pixel and returns a linear image. Logs lifecycle; no disk writes. |
| `ray_color` | Public. Collects lights then delegates for one ray; consumes caller RNG and returns radiance. |
| `ray_color_inner` | Private. Bounded bounce loop with emission, explicit lighting/MIS, and material scatter; consumes RNG. |
| `direct_area_light` | Private. Samples one emissive triangle, tests visibility, returns diffuse direct estimate. |
| `direct_environment` | Private. Importance-samples environment, tests visibility, applies MIS. |
| `power_heuristic` | Private. Returns squared-PDF MIS weight, zero for invalid denominator. |
| `scatter` | Private. Produces next ray/attenuation/material flags for supported materials; may return `None` to terminate. |
| `material_at_hit` | Private. Resolves textured diffuse to sampled constant diffuse; other materials pass through. |
| `reflect` | Private specular reflection vector helper. |
| `refract` | Private Snell refraction vector helper. |
| `reflectance` | Private Schlick approximation. |
| `random_unit_vector` | Private rejection-based/unit-sphere direction sampler consuming RNG. |
| `random_in_unit_sphere` | Private rejection sampler consuming RNG until inside the sphere. |

`intersect.rs` owns brute-force hit queries.

| `intersect_sphere` | Public. Returns nearest sphere hit beyond `min_distance`; orients normal and records material. |
| `intersect_triangle` | Public. Triangle query without optional attributes; delegates to internal routine. |
| `intersect_triangle_with_attributes` | Private. Möller–Trumbore query with smooth-normal/UV interpolation and finite max bound. |
| `intersect_scene` | Public. Scans every sphere/triangle and returns the closest hit. Principal callers: integrator and shadow rays. |

`light.rs` owns emissive-triangle area sampling.

| `AreaLights::collect` | Public. Builds cumulative areas for positive-emission triangles; skips degenerate area. |
| `len` | Public count accessor. |
| `is_empty` | Public emptiness accessor. |
| `sample` | Public. Selects by area and samples barycentrics; consumes RNG and returns point/normal/emission/area PDF or `None`. |

### `toaster-gpu`

`animation.rs` owns render schedules and file naming.

| Function | Visibility and contract |
| --- | --- |
| `AnimationConfig::single_frame` | Public. Creates one frame at time zero with no FPS. |
| `from_duration` | Public. Validates FPS/finite positive duration and uses `ceil(fps*duration)`; errors on overflow. |
| `from_frame_count` | Public. Validates nonzero FPS/count. |
| `indefinite` | Public. Validates FPS and creates no frame limit. |
| `with_loop_duration` | Public. Requires an FPS and finite positive loop duration; returns updated schedule. |
| `frame_limit` | Public optional-count accessor. |
| `time_for_frame` | Public. Returns `frame/fps`, optionally modulo loop duration; single frame returns zero. |
| `fps` | Public optional-rate accessor. |
| `loop_duration` | Public optional-loop accessor. |
| `validate_fps` | Private nonzero check. |
| `frame_output_path` | Public. Returns unchanged path for one frame or appends a zero-padded frame suffix for sequences. |

`device.rs` owns adapter/device creation.

| `GpuAdapterInfo::from` | Private trait conversion from wgpu adapter strings. |
| `create_gpu_context` | Public async. Requests default high-performance-compatible adapter/device and returns device/queue/info; errors if unavailable/request fails. Logs selected GPU. |

`buffers.rs` owns allocation and initial uploads.

| `create_pixel_buffers` | Public. Allocates output/readback/uniform buffers for the gradient shader; output size derives from dimensions. |
| `create_scene_gpu_buffers` | Public. Allocates and uploads all path-tracer buffers; substitutes dummy entries for empty logical arrays. Called once per render session. |

`pipeline.rs` owns layouts and compilation.

| `create_bind_group_layout` | Public. Creates the two-binding gradient layout. |
| `create_bind_group` | Public. Binds gradient output and params to that layout. |
| `create_pathtrace_bind_group_layout` | Public. Creates the ten-binding active path-tracer layout. |
| `create_pathtrace_bind_group` | Public. Binds every scene/output buffer in fixed order. |
| `load_shader` | Public. Creates a WGSL shader module; validation errors surface during wgpu pipeline setup. |
| `create_pipeline` | Public. Creates a compute pipeline targeting WGSL `main`. |

`dispatch.rs` and `readback.rs` own synchronous execution.

| `dispatch_compute_2d` | Public. Encodes dispatch, copies output to readback, submits, and blocks for completion; returns queue/map-adjacent errors. Workgroups are ceiling dimensions/8. |
| `readback_pixels` | Public. Maps the complete buffer, waits, copies aligned bytes to `[f32;4]`, unmaps, and returns pixels; map failure is contextual. |

`scene_upload.rs` owns packing.

| Function | Visibility and contract |
| --- | --- |
| `load_scene_gpu` | Public. Loads JSON then fully packs it. |
| `scene_to_gpu` | Public. Packs scene plus static texture/environment pixels. |
| `scene_to_gpu_frame` | Crate. Packs mutable frame data without duplicating static pixel arrays. |
| `scene_to_gpu_inner` | Private. Validates renderer requirements/count widths/attribute alignment, creates camera/geometry/material/lights/environment params. |
| `make_camera` | Public. Computes GPU pinhole basis; assumes validated nondegenerate inputs. |
| `material_to_gpu` | Private. Maps material tags/parameters and resolves texture atlas metadata; errors on bad indices. |
| `vec4` | Private alignment helper expanding `Vec3` with zero W. |
| `lights_to_gpu` | Private. Collects positive-emission spheres/triangles and cumulative areas; errors on index conversion. |
| `is_emissive` | Private positive-strength predicate. |

`image_output.rs` owns the shared display conversion.

| `save_pixels_to_png` | Public. Converts float pixels then saves PNG. |
| `save_rgba_image_to_png` | Public. Encodes an existing RGBA image, returning filesystem/encoder errors. |
| `encode_pixels_to_jpeg` | Public. Converts float pixels then encodes JPEG at caller quality. |
| `encode_rgba_image_to_jpeg` | Public. Drops alpha through RGB conversion and returns complete JPEG bytes; rejects encoder errors. |
| `pixels_to_rgba_image` | Public. Validates exact dimensions/count and converts linear RGB to shared RGBA8. |
| `f32_to_u8` | Private. Sanitizes/clamps, applies square-root display transform, quantizes a channel. |

`video_output.rs` owns FFmpeg lifecycle.

| `FfmpegVideoWriter::start` | Crate. Resolves `TOASTER_FFMPEG` or `ffmpeg`, validates MP4 path, starts configured child and captures stdin/stderr. |
| `start_with_program` | Private testable process constructor with explicit executable. |
| `write_frame` | Crate. Writes a complete packed RGBA frame; errors if pipe is closed/encoder exits. |
| `finish` | Crate. Closes input, waits, and converts nonzero exit/stderr to an error. |
| `Drop::drop` | Private trait method. Closes stdin, kills, and waits for an encoder not explicitly finished. Side effect: process termination. |

`gradient.rs` is a diagnostic path.

| `default_render_params` | Private fixed 800×600 settings constructor. |
| `render_gradient` | Public async. Creates context/buffers/pipeline, dispatches, reads back, and writes PNG; logs stages and returns setup/output errors. |

`path_tracer.rs` owns schedules, sinks, benchmarks, and the shared loop.

| Function | Visibility and contract |
| --- | --- |
| `GpuBenchmarkConfig::new` | Public. Requires measured frames >0 and nonoverflowing total. |
| `warmup_frames`, `measured_frames` | Public count accessors. |
| `ProgressiveRenderConfig::new` | Public. Requires target >0. |
| `target_samples` | Public target accessor. |
| `FrameSink::deliver` | Public required trait method. Receives a borrowed completed image synchronously and may fail the render. |
| `FrameSink::should_continue` | Public default cancellation method returning true; queried between frames. |
| `FrameSink::samples_for_frame` | Public default returning no override; queried before dispatch. |
| `PngFrameSink::deliver` | Private trait implementation writing one PNG and logging its path. |
| `VideoFrameSink::finish` | Private. Takes/finalizes its writer exactly once. |
| `VideoFrameSink::deliver` | Private trait implementation writing a complete raw frame to FFmpeg. |
| `BenchmarkFrameSink::deliver` | Private trait implementation discarding warmups and cloning the latest measured image. |
| `render_scene_gpu` | Public async one-frame PNG wrapper. |
| `render_scene_gpu_animation` | Public async finite PNG wrapper; errors for indefinite schedule. |
| `render_scene_gpu_video` | Public async finite video wrapper; requires FPS/frame limit and always attempts encoder finalization. |
| `render_scene_gpu_animation_with_sink` | Public async path-loading sink wrapper; validates pacing before loading. |
| `render_gpu_animation_with_sink` | Public async already-loaded independent-frame entry point. |
| `render_gpu_progressive_with_sink` | Public async static progressive entry; rejects tracks/loop duration and holds after target. |
| `benchmark_gpu_scene` | Public async benchmark entry; constructs fixed schedule/sink, enforces requested measurement count, returns summary/image. |
| `render_gpu_with_sink` | Private async shared single-setup loop. Owns setup, evaluation/upload, sample selection, dispatch/readback/conversion/delivery, timing, pacing, and cancellation. |
| `hold_completed_preview` | Private. Polls cancellation up to a finite deadline or indefinitely at 50 ms without GPU dispatch. |
| `frame_deadline_offset` | Private. Computes exact elapsed duration `frame/fps`; assumes nonzero validated FPS. |
| `progressive_batch` | Private. Validates counts/order and returns remaining-clamped batch or `None` at completion. |
| `validate_pacing` | Private. Requires FPS for real-time mode. |

`gpu_types.rs` contains data-only host layout records; its public fields are individually documented in Rustdoc and mapped in [shader_reference.md](shader_reference.md).

### `toaster-server`

`lib.rs` owns listener and latest state.

| Function | Visibility and contract |
| --- | --- |
| `serve` | Public async. Creates a publisher then binds/serves until failure/cancellation. Network side effect. |
| `serve_with_publisher` | Public async. Binds and delegates with supplied state. |
| `bind` | Public async. Resolves host/port and creates `TcpListener`; returns address availability errors before GPU work. |
| `serve_listener` | Public async. Logs local address, builds router, and serves listener. |
| `FramePublisher::new` | Public. Creates independent watch channels initialized with no JPEG/status. |
| `publish_jpeg` | Public. Replaces latest JPEG while preserving status behavior; wakes receivers, never queues. |
| `publish_frame` | Public. Replaces latest JPEG and status; slow receivers observe only newest values. |
| `subscribe` | Crate. Creates a JPEG watch receiver for `/stream`. |
| `latest_status` | Crate. Clones current optional status for `/status`. |
| `Default::default` | Private trait method delegating to `new`. |

`routes.rs` owns HTTP representation.

| `router` | Public. Installs state, four GET routes, and request tracing (not individual MJPEG chunks). |
| `index` | Private async. Returns static browser HTML containing the stream image/status script. |
| `healthz` | Private async. Returns `ok` with HTTP 200. |
| `status` | Private async. Returns current optional `PreviewStatus` JSON. |
| `stream` | Private async. Emits an initial/current JPEG then watch updates as multipart body; sets MJPEG/no-cache headers. |
| `mjpeg_part` | Private. Builds one complete `--frame` JPEG part with content type/length/CRLF. |

### `toaster-cli`

`cli.rs` owns parsing only.

| `LogLevel::directive` | Public within binary. Returns target-filter directive used by tracing initialization. |
| `parse_nonnegative_f64` | Private Clap parser rejecting NaN/infinity/negative thresholds. |
| `parse_positive_f32` | Private Clap parser rejecting NaN/infinity/nonpositive durations. |

`logging.rs` owns process subscriber setup.

| `init` | Public within binary. Selects explicit level, `RUST_LOG`, or info default; installs text/JSON stderr layer once and returns filter/subscriber errors. |

`main.rs` owns command orchestration.

| Function | Visibility and contract |
| --- | --- |
| `AdaptiveSampling::new` | Private. Validates ordered bounds/nonzero FPS, clamps initial samples, computes frame budget. |
| `observe` | Private. Mutates next samples using budget thresholds/limited step changes; returns UI status phrase. |
| `ServerFrameSink::deliver` | Private trait implementation. JPEG-encodes, computes effective FPS/adaptation/progress, replaces server state, logs publication. |
| `should_continue` | Private trait implementation reading shared atomic stop flag. |
| `samples_for_frame` | Private trait implementation returning adaptive or fixed batch override. |
| `main` | Private process entry. Parses, initializes logs, invokes `run`, logs failure, returns success/failure exit code. |
| `run` | Private. Matches every command, loads/configures dependencies, writes requested outputs, and propagates errors. `info` writes stdout. |
| `render_stream_preview` | Private. Creates runtime/listener/tasks/sink, runs animated or progressive GPU entry, and guarantees task cleanup. |
| `resolve_progressive_preview` | Private. Enforces flag/static-scene restrictions and derives nonzero ordered batch/target defaults. |
| `resolve_adaptive_sampling` | Private. Enforces flag/bounds/progressive ceiling relationships and creates controller. |
| `resolve_preview_animation` | Private. Builds finite/indefinite FPS schedule and optional loop. |
| `resolve_animation` | Private. Enforces valid still/duration/frame-count CLI shapes. |
| `apply_overrides` | Private. Replaces supplied render settings and rejects zero resolved values. |

`benchmark.rs` owns report data and comparison.

| Function | Visibility and contract |
| --- | --- |
| `build_report` | Public within binary. Requires measured frames, reads system time, converts timings/counts/adapter/image hash into schema v1. |
| `save_reference_image` | Public within binary. Creates parent directories and writes final PNG. |
| `write_report` | Public within binary. Creates parent directories and writes pretty JSON plus newline. |
| `read_report` | Public within binary. Reads/deserializes JSON with path context. |
| `compare_reports` | Public within binary. Validates limit/schema/settings, computes deltas, GPU/driver/checksum flags, and threshold decision. Threshold requires matching GPU and positive baseline. |
| `FrameTimingReport::from` | Private trait conversion of durations to milliseconds. |
| `TimingSummary::from_frames` | Private. Summarizes all stages; errors on empty/invalid data. |
| `statistics` | Private. Sorts finite nonnegative values and computes min/mean/conventional median/nearest-rank p95/max. |
| `milliseconds` | Private duration conversion. |
| `rgba_sha256` | Private lowercase SHA-256 formatter. |
| `emissive_object_count` | Private. Counts positive-emission spheres and triangles with valid material references. |
| `ensure_compatible_scene` | Private. Requires identical resolved settings and geometry/material/light counts; path may differ. |
| `gpu_identity` | Private. Returns name/backend/device-type tuple. |
| `percent_change` | Private. Computes relative percentage only for positive baseline. |
| `create_parent_directory` | Private. Creates a nonempty parent hierarchy for output paths. |

### `toaster-bvh`

`aabb.rs`, `bvh.rs`, and `flatten.rs` contain no production functions or types. They document the intended contract only: finite primitive AABBs/ray-box tests; hierarchy construction over stable primitive bounds; flattening for CPU traversal and GPU storage. No renderer depends on this crate today, and this document intentionally does not invent incoming symbols.

## Tests and validation ownership

- Core tests cover camera validation/rays, color conversion, buffer access/output, and ray evaluation.
- Asset tests cover glTF geometry/material/texture import and error shapes.
- Scene tests cover JSON defaults/validation/assets, animation composition/interpolation, texture sampling, and environment evaluation/importance PDFs.
- CPU tests cover intersections, lights, materials, and rendered behavior.
- GPU hardware-independent tests cover schedules, image conversion/JPEG decoding, layouts, progressive batch arithmetic, sink semantics, benchmark configuration/timings, and FFmpeg process behavior. GPU-node smoke tests remain manual.
- Server tests cover HTML, health/status, MJPEG framing/content type, complete JPEG parts, and latest-value replacement.
- CLI tests cover every command/flag shape, progressive/adaptive invalid combinations, overrides, report statistics/serialization/comparison, and logging choices.

Required contributor validation is in [CONTRIBUTING.md](../CONTRIBUTING.md). Documentation-specific validation is `./scripts/check_docs.sh`.
