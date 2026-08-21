//! Compute path-tracing pipeline.
use anyhow::{anyhow, ensure, Context, Result};
use image::RgbaImage;
use std::path::Path;
use std::time::{Duration, Instant};
use toaster_scene::{
    AnimationEvaluator, NamedCounterSnapshot, SceneChanges, SceneEvaluationTimings, SceneEvaluator,
};

use crate::animation::{frame_output_path, AnimationConfig};
use crate::buffers::create_scene_gpu_buffers;
use crate::device::{create_gpu_context, GpuAdapterInfo};
use crate::dispatch::{dispatch_compute_2d, ComputeDispatch};
use crate::image_output::{pixels_to_rgba_image, save_rgba_image_to_png};
use crate::pipeline::{
    create_pathtrace_bind_group, create_pathtrace_bind_group_layout, create_pipeline, load_shader,
};
use crate::readback::readback_pixels;
use crate::scene_upload::{scene_to_gpu, scene_to_gpu_frame_with_timings};
use crate::video_output::FfmpegVideoWriter;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Controls whether completed frames are produced as fast as possible or on deadlines.
pub enum FramePacing {
    /// Begin the next frame immediately after the preceding frame is delivered.
    Unpaced,
    /// Sleep until each `frame_index / fps` deadline when rendering is ahead.
    RealTime,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// CPU wall-clock durations for the stages of one GPU-rendered frame.
pub struct GpuFrameTimings {
    /// Time spent cloning the base scene and applying animation tracks.
    pub animation_evaluation: Duration,
    /// Time spent resetting, replaying, or incrementally stepping physics.
    pub physics_evaluation: Duration,
    /// Time spent applying neutral evaluator updates to render geometry.
    pub geometry_update: Duration,
    /// Time spent rebuilding the direct-light sampling list.
    pub light_rebuild: Duration,
    /// Time spent rebuilding and flattening the mixed primitive BVH.
    pub bvh_rebuild: Duration,
    /// Time spent writing mutable buffers into the GPU queue.
    pub gpu_upload: Duration,
    /// Time spent evaluating the scene and uploading mutable buffers.
    pub scene_update_upload: Duration,
    /// Time from dispatch preparation through synchronous GPU completion.
    pub dispatch_wait: Duration,
    /// Time spent mapping and copying the readback buffer.
    pub readback: Duration,
    /// Time spent converting linear floating-point pixels to RGBA8.
    pub conversion: Duration,
    /// Time spent delivering the converted frame to its output sink.
    pub output: Duration,
    /// Total frame time through output delivery.
    pub total: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Warmup and measurement counts for a static, independent-frame benchmark.
pub struct GpuBenchmarkConfig {
    warmup_frames: u32,
    measured_frames: u32,
}

impl GpuBenchmarkConfig {
    /// Validates and creates a benchmark configuration.
    ///
    /// At least one measured frame is required, and the combined count must fit
    /// in a `u32` schedule.
    pub fn new(warmup_frames: u32, measured_frames: u32) -> Result<Self> {
        ensure!(
            measured_frames > 0,
            "benchmark measured frame count must be greater than zero"
        );
        warmup_frames
            .checked_add(measured_frames)
            .context("benchmark frame count overflowed")?;
        Ok(Self {
            warmup_frames,
            measured_frames,
        })
    }

    /// Returns the number of frames discarded before measurement.
    pub fn warmup_frames(self) -> u32 {
        self.warmup_frames
    }

    /// Returns the number of frames retained in the result.
    pub fn measured_frames(self) -> u32 {
        self.measured_frames
    }
}

#[derive(Clone, Debug)]
/// Results from one prepared-device GPU benchmark run.
pub struct GpuBenchmarkResult {
    /// Identity and driver metadata for the selected adapter.
    pub adapter: GpuAdapterInfo,
    /// One-time context, buffer, bind-group, shader, and pipeline setup time.
    pub setup_time: Duration,
    /// Per-frame timings after warmup.
    pub frames: Vec<GpuFrameTimings>,
    /// Converted image from the final measured frame.
    pub final_image: RgbaImage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Validated target for a static progressive accumulation.
pub struct ProgressiveRenderConfig {
    target_samples: u32,
}

impl ProgressiveRenderConfig {
    /// Creates a progressive configuration with a nonzero sample target.
    pub fn new(target_samples: u32) -> Result<Self> {
        ensure!(
            target_samples > 0,
            "progressive target samples must be greater than zero"
        );
        Ok(Self { target_samples })
    }

    /// Returns the exact accumulated sample count at which rendering stops.
    pub fn target_samples(self) -> u32 {
        self.target_samples
    }
}

/// Borrowed view of a converted frame passed to a [`FrameSink`].
pub struct CompletedFrame<'a> {
    /// Zero-based dispatch index in this render session.
    pub index: u32,
    /// Scene animation time used for this frame, in seconds.
    pub time_seconds: f32,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Samples rendered by this dispatch.
    pub samples: u32,
    /// Total samples represented by `image` after this dispatch.
    pub accumulated_samples: u32,
    /// Maximum path depth used by this dispatch.
    pub max_bounces: u32,
    /// Detailed wall-clock timings for this dispatch.
    pub timings: GpuFrameTimings,
    /// Total time through RGBA conversion, retained for compatibility.
    pub render_time: Duration,
    /// Shared converted image; valid only for the duration of delivery.
    pub image: &'a RgbaImage,
    /// Renderer-neutral named counters corresponding to this evaluated frame.
    pub counters: &'a NamedCounterSnapshot,
}

/// Consumer invoked synchronously for every completed GPU frame.
///
/// Sinks may write images, encode video, publish previews, record timings, or
/// request cancellation. Delivery runs between frames and therefore applies
/// backpressure to the single render loop.
pub trait FrameSink {
    /// Consumes one completed-frame view.
    fn deliver(&mut self, frame: CompletedFrame<'_>) -> Result<()>;

    /// Returns whether rendering should proceed to another frame.
    fn should_continue(&self) -> bool {
        true
    }

    /// Optionally overrides the scene sample count for the next dispatch.
    fn samples_for_frame(&self) -> Option<u32> {
        None
    }

    /// Receives final timings after output delivery has completed.
    fn record_timings(&mut self, _frame_index: u32, _timings: GpuFrameTimings) {}
}

/// Frame sink that persists finite output as one or more PNG files.
struct PngFrameSink<'a> {
    /// Requested base output path.
    out_path: &'a Path,
    /// Total scheduled frames, used to select naming behavior.
    frame_count: u32,
}

impl FrameSink for PngFrameSink<'_> {
    /// Saves a completed image using the animation frame naming convention.
    fn deliver(&mut self, frame: CompletedFrame<'_>) -> Result<()> {
        let save_start = Instant::now();
        let frame_path = frame_output_path(self.out_path, frame.index, self.frame_count);
        save_rgba_image_to_png(frame.image, &frame_path)?;

        tracing::debug!(
            frame.index,
            duration_ms = save_start.elapsed().as_secs_f64() * 1000.0,
            "saved PNG frame"
        );
        tracing::info!(frame.index, path = %frame_path.display(), "wrote PNG frame");
        Ok(())
    }
}

/// Frame sink that forwards converted frames to an FFmpeg process.
struct VideoFrameSink {
    /// Optional so finalization can take ownership exactly once.
    writer: Option<FfmpegVideoWriter>,
}

impl VideoFrameSink {
    /// Finalizes the encoder and propagates its exit status.
    fn finish(mut self) -> Result<()> {
        self.writer
            .take()
            .context("video writer is unavailable")?
            .finish()
    }
}

impl FrameSink for VideoFrameSink {
    /// Writes one completed image into the raw-video pipe.
    fn deliver(&mut self, frame: CompletedFrame<'_>) -> Result<()> {
        let output_start = Instant::now();
        self.writer
            .as_mut()
            .context("video writer is unavailable")?
            .write_frame(frame.image)?;
        tracing::debug!(
            frame.index,
            duration_ms = output_start.elapsed().as_secs_f64() * 1000.0,
            "submitted video frame"
        );
        Ok(())
    }
}

/// Frame sink that discards warmup data and retains benchmark measurements.
struct BenchmarkFrameSink {
    /// Number of leading deliveries to ignore.
    warmup_frames: u32,
    /// Measured frame timing records.
    timings: Vec<GpuFrameTimings>,
    /// Image from the most recent measured frame.
    final_image: Option<RgbaImage>,
}

impl FrameSink for BenchmarkFrameSink {
    /// Records timing and pixels only after the warmup boundary.
    fn deliver(&mut self, frame: CompletedFrame<'_>) -> Result<()> {
        if frame.index >= self.warmup_frames {
            self.final_image = Some(frame.image.clone());
        }
        Ok(())
    }

    fn record_timings(&mut self, frame_index: u32, timings: GpuFrameTimings) {
        if frame_index >= self.warmup_frames {
            self.timings.push(timings);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Determines whether scene animation and frame-index seed advance per frame.
enum FrameMode {
    /// Evaluate animation time normally and use the sequential frame seed.
    Animated,
    /// Reuse time zero and frame seed zero for comparable benchmark samples.
    StaticIndependent,
}

/// Internal render-loop metadata needed by benchmark callers.
struct GpuRenderSummary {
    /// Selected adapter identity.
    adapter: GpuAdapterInfo,
    /// One-time initialization duration.
    setup_time: Duration,
}

/// Renders a scene's time-zero frame to a PNG.
pub async fn render_scene_gpu(scene_path: &Path, out_path: &Path) -> Result<()> {
    render_scene_gpu_animation(scene_path, out_path, AnimationConfig::single_frame()).await
}

/// Renders a finite animation schedule to numbered PNG files.
///
/// The schedule must have a frame limit because file output cannot represent an
/// indefinite render.
pub async fn render_scene_gpu_animation(
    scene_path: &Path,
    out_path: &Path,
    animation: AnimationConfig,
) -> Result<()> {
    let source_scene = toaster_scene::load_scene(scene_path)?;
    ensure!(
        !source_scene.physics.is_some_and(|physics| physics.enabled),
        "path-based GPU helpers do not evaluate enabled physics; use an evaluator-driven entrypoint"
    );
    let mut evaluator = AnimationEvaluator::new(source_scene);
    render_evaluated_gpu_animation(&mut evaluator, out_path, animation).await
}

/// Renders a finite evaluator-driven schedule to numbered PNG files.
pub async fn render_evaluated_gpu_animation(
    evaluator: &mut dyn SceneEvaluator,
    out_path: &Path,
    animation: AnimationConfig,
) -> Result<()> {
    let frame_count = animation
        .frame_limit()
        .context("PNG animation rendering requires a finite frame count")?;
    let mut sink = PngFrameSink {
        out_path,
        frame_count,
    };
    render_evaluated_gpu_animation_with_sink(evaluator, animation, FramePacing::Unpaced, &mut sink)
        .await
}

/// Renders a finite animation and streams its RGBA frames into an MP4 encoder.
pub async fn render_scene_gpu_video(
    scene_path: &Path,
    video_path: &Path,
    animation: AnimationConfig,
) -> Result<()> {
    let source_scene = toaster_scene::load_scene(scene_path)?;
    ensure!(
        !source_scene.physics.is_some_and(|physics| physics.enabled),
        "path-based GPU helpers do not evaluate enabled physics; use an evaluator-driven entrypoint"
    );
    let mut evaluator = AnimationEvaluator::new(source_scene);
    render_evaluated_gpu_video(&mut evaluator, video_path, animation).await
}

/// Renders a finite evaluator-driven schedule into an MP4 encoder.
pub async fn render_evaluated_gpu_video(
    evaluator: &mut dyn SceneEvaluator,
    video_path: &Path,
    animation: AnimationConfig,
) -> Result<()> {
    let fps = animation
        .fps()
        .context("video export requires an animated render with --fps")?;
    ensure!(
        animation.frame_limit().is_some(),
        "video export requires a finite --duration or --frames"
    );
    let width = evaluator.source_scene().render.width;
    let height = evaluator.source_scene().render.height;
    let mut sink = VideoFrameSink {
        writer: Some(FfmpegVideoWriter::start(video_path, width, height, fps)?),
    };
    let render_result = render_evaluated_gpu_animation_with_sink(
        evaluator,
        animation,
        FramePacing::Unpaced,
        &mut sink,
    )
    .await;
    let finish_result = sink.finish();

    match (render_result, finish_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(render_error), Ok(())) => Err(render_error),
        (Ok(()), Err(finish_error)) => Err(finish_error),
        (Err(render_error), Err(finish_error)) => Err(anyhow!(
            "{render_error:#}; video finalization also failed: {finish_error:#}"
        )),
    }
}

/// Loads a scene and renders its animation through a caller-provided sink.
pub async fn render_scene_gpu_animation_with_sink(
    scene_path: &Path,
    animation: AnimationConfig,
    pacing: FramePacing,
    sink: &mut dyn FrameSink,
) -> Result<()> {
    validate_pacing(animation, pacing)?;
    let source_scene = toaster_scene::load_scene(scene_path)?;
    ensure!(
        !source_scene.physics.is_some_and(|physics| physics.enabled),
        "path-based GPU helpers do not evaluate enabled physics; use an evaluator-driven entrypoint"
    );
    render_gpu_animation_with_sink(&source_scene, animation, pacing, sink).await
}

/// Renders an already loaded scene as independent animation frames.
pub async fn render_gpu_animation_with_sink(
    source_scene: &toaster_scene::Scene,
    animation: AnimationConfig,
    pacing: FramePacing,
    sink: &mut dyn FrameSink,
) -> Result<()> {
    ensure!(
        !source_scene.physics.is_some_and(|physics| physics.enabled),
        "scene-only GPU helpers do not evaluate enabled physics; use an evaluator-driven entrypoint"
    );
    let mut evaluator = AnimationEvaluator::new(source_scene.clone());
    render_evaluated_gpu_animation_with_sink(&mut evaluator, animation, pacing, sink).await
}

/// Renders frames supplied by a renderer-neutral scene evaluator.
pub async fn render_evaluated_gpu_animation_with_sink(
    evaluator: &mut dyn SceneEvaluator,
    animation: AnimationConfig,
    pacing: FramePacing,
    sink: &mut dyn FrameSink,
) -> Result<()> {
    render_gpu_with_sink(
        evaluator,
        animation,
        pacing,
        None,
        FrameMode::Animated,
        sink,
    )
    .await
    .map(|_| ())
}

/// Accumulates a static scene in linear HDR batches and delivers each update.
///
/// Animation tracks and loop durations are rejected. Once the target is reached,
/// a real-time preview remains alive until its finite deadline or cancellation.
pub async fn render_gpu_progressive_with_sink(
    source_scene: &toaster_scene::Scene,
    schedule: AnimationConfig,
    pacing: FramePacing,
    config: ProgressiveRenderConfig,
    sink: &mut dyn FrameSink,
) -> Result<()> {
    ensure!(
        schedule.loop_duration().is_none(),
        "progressive preview does not support an animation loop duration"
    );
    ensure!(
        !source_scene.physics.is_some_and(|physics| physics.enabled),
        "progressive preview does not support physics-enabled scenes"
    );
    ensure!(
        source_scene.animation.tracks.is_empty(),
        "progressive preview requires a scene without animation tracks"
    );
    let mut evaluator = AnimationEvaluator::new(source_scene.clone());
    render_gpu_with_sink(
        &mut evaluator,
        schedule,
        pacing,
        Some(config),
        FrameMode::Animated,
        sink,
    )
    .await
    .map(|_| ())
}

/// Benchmarks static independent frames using one GPU setup.
///
/// Warmup frames are rendered but omitted from [`GpuBenchmarkResult::frames`].
pub async fn benchmark_gpu_scene(
    source_scene: &toaster_scene::Scene,
    config: GpuBenchmarkConfig,
) -> Result<GpuBenchmarkResult> {
    ensure!(
        !source_scene.physics.is_some_and(|physics| physics.enabled),
        "scene-only GPU benchmarks do not evaluate enabled physics; use an evaluator-driven benchmark"
    );
    let mut evaluator = AnimationEvaluator::new(source_scene.clone());
    benchmark_gpu_evaluator(&mut evaluator, config).await
}

/// Benchmarks evaluator-produced time-zero frames using one GPU setup.
pub async fn benchmark_gpu_evaluator(
    evaluator: &mut dyn SceneEvaluator,
    config: GpuBenchmarkConfig,
) -> Result<GpuBenchmarkResult> {
    let total_frames = config
        .warmup_frames()
        .checked_add(config.measured_frames())
        .context("benchmark frame count overflowed")?;
    let schedule = AnimationConfig::from_frame_count(1, total_frames)?;
    let measured_capacity = usize::try_from(config.measured_frames())
        .context("benchmark measured frame count is too large")?;
    let mut sink = BenchmarkFrameSink {
        warmup_frames: config.warmup_frames(),
        timings: Vec::with_capacity(measured_capacity),
        final_image: None,
    };
    let summary = render_gpu_with_sink(
        evaluator,
        schedule,
        FramePacing::Unpaced,
        None,
        FrameMode::StaticIndependent,
        &mut sink,
    )
    .await?;
    ensure!(
        sink.timings.len() == measured_capacity,
        "GPU benchmark completed without all requested measurements"
    );
    let final_image = sink
        .final_image
        .context("GPU benchmark did not produce a measured image")?;

    Ok(GpuBenchmarkResult {
        adapter: summary.adapter,
        setup_time: summary.setup_time,
        frames: sink.timings,
        final_image,
    })
}

/// Implements the shared single-setup render loop for all sink-driven modes.
async fn render_gpu_with_sink(
    evaluator: &mut dyn SceneEvaluator,
    animation: AnimationConfig,
    pacing: FramePacing,
    progressive: Option<ProgressiveRenderConfig>,
    frame_mode: FrameMode,
    sink: &mut dyn FrameSink,
) -> Result<GpuRenderSummary> {
    validate_pacing(animation, pacing)?;

    let initial_evaluated = evaluator.evaluate(animation.evaluation_request(0))?;
    let initial_counters = initial_evaluated.counters;
    let initial_scene = initial_evaluated.scene;
    let display = initial_scene.display;
    let mut scene = scene_to_gpu(&initial_scene)?;
    let initial_counts = (
        scene.spheres.len(),
        scene.triangles.len(),
        scene.triangle_attributes.len(),
        scene.materials.len(),
        scene.lights.len(),
        scene.bvh_primitives.len(),
    );
    let bvh_node_capacity = scene
        .spheres
        .len()
        .checked_add(scene.triangles.len())
        .and_then(|count| count.checked_mul(2))
        .and_then(|count| count.checked_sub(1))
        .context("evaluated scene BVH capacity overflowed")?;
    let total_start = Instant::now();

    tracing::info!(
        width = scene.params.width,
        height = scene.params.height,
        samples = scene.params.samples,
        max_bounces = scene.params.max_bounces,
        spheres = scene.params.sphere_count,
        triangles = scene.params.triangle_count,
        materials = scene.params.material_count,
        lights = scene.params.light_count,
        "starting GPU path trace"
    );

    let setup_start = Instant::now();

    let context = create_gpu_context().await?;
    tracing::debug!("created GPU context");

    let buffers = create_scene_gpu_buffers(&context.device, &scene);
    tracing::debug!("created scene GPU buffers");

    let bind_group_layout = create_pathtrace_bind_group_layout(&context.device);
    let bind_group = create_pathtrace_bind_group(&context.device, &bind_group_layout, &buffers);

    let shader = load_shader(
        &context.device,
        "Pathtrace Shader",
        include_str!("../../../shaders/pathtrace_spheres.wgsl"),
    );

    let pipeline = create_pipeline(&context.device, &shader, &bind_group_layout);
    let setup_time = setup_start.elapsed();

    tracing::info!(
        duration_ms = setup_time.as_secs_f64() * 1000.0,
        "completed GPU setup"
    );

    let pacing_started_at = Instant::now();
    let mut frame = 0_u32;
    let mut accumulated_samples = 0_u32;

    while animation
        .frame_limit()
        .is_none_or(|frame_limit| frame < frame_limit)
    {
        if !sink.should_continue() {
            tracing::info!(frame.index = frame, "GPU render cancelled before frame");
            break;
        }

        let frame_start = Instant::now();
        let scene_update_start = Instant::now();
        let request = if progressive.is_some() || frame_mode == FrameMode::StaticIndependent {
            toaster_scene::EvaluationRequest {
                time_seconds: 0.0,
                loop_cycle: 0,
            }
        } else {
            animation.evaluation_request(frame)
        };
        let time_seconds = request.time_seconds;
        let (next_scene, changes, evaluation_timings, packing_timings, counters) = if progressive
            .is_some()
        {
            let cached_lights = Some((scene.lights.as_slice(), scene.params.total_light_weight));
            let (next_scene, packing_timings) =
                scene_to_gpu_frame_with_timings(&initial_scene, cached_lights)?;
            (
                next_scene,
                SceneChanges::default(),
                SceneEvaluationTimings::default(),
                packing_timings,
                initial_counters.clone(),
            )
        } else {
            let evaluated = evaluator.evaluate(request)?;
            let changes = evaluated.changes;
            let cached_lights = (!changes.emissive_geometry)
                .then_some((scene.lights.as_slice(), scene.params.total_light_weight));
            let (next_scene, packing_timings) =
                scene_to_gpu_frame_with_timings(&evaluated.scene, cached_lights)?;
            (
                next_scene,
                changes,
                evaluated.timings,
                packing_timings,
                evaluated.counters,
            )
        };
        ensure!(
            (
                next_scene.spheres.len(),
                next_scene.triangles.len(),
                next_scene.triangle_attributes.len(),
                next_scene.materials.len(),
                next_scene.lights.len(),
                next_scene.bvh_primitives.len(),
            ) == initial_counts,
            "evaluated scenes must preserve GPU geometry, material, light, and BVH primitive counts"
        );
        ensure!(
            next_scene.bvh_nodes.len() <= bvh_node_capacity,
            "evaluated scene BVH exceeds the preallocated GPU node capacity"
        );
        scene = next_scene;
        let requested_samples = sink.samples_for_frame().unwrap_or(scene.params.samples);
        ensure!(
            requested_samples > 0,
            "frame sink sample count must be greater than zero"
        );
        scene.params.samples = match progressive {
            Some(config) => progressive_batch(
                requested_samples,
                accumulated_samples,
                config.target_samples(),
            )?
            .context("progressive render is already complete")?,
            None => requested_samples,
        };
        scene.params.frame_index = render_frame_seed(frame, frame_mode);
        scene.params.accumulated_samples = if progressive.is_some() {
            accumulated_samples
        } else {
            0
        };

        let upload_start = Instant::now();
        if changes.camera {
            context
                .queue
                .write_buffer(&buffers.camera, 0, bytemuck::bytes_of(&scene.camera));
        }
        context
            .queue
            .write_buffer(&buffers.params, 0, bytemuck::bytes_of(&scene.params));
        if changes.spheres && !scene.spheres.is_empty() {
            context
                .queue
                .write_buffer(&buffers.spheres, 0, bytemuck::cast_slice(&scene.spheres));
        }
        if changes.triangles && !scene.triangles.is_empty() {
            context.queue.write_buffer(
                &buffers.triangles,
                0,
                bytemuck::cast_slice(&scene.triangles),
            );
            context.queue.write_buffer(
                &buffers.triangle_attributes,
                0,
                bytemuck::cast_slice(&scene.triangle_attributes),
            );
        }
        if changes.emissive_geometry && !scene.lights.is_empty() {
            context
                .queue
                .write_buffer(&buffers.lights, 0, bytemuck::cast_slice(&scene.lights));
        }
        if (changes.spheres || changes.triangles) && !scene.bvh_nodes.is_empty() {
            context.queue.write_buffer(
                &buffers.bvh_nodes,
                0,
                bytemuck::cast_slice(&scene.bvh_nodes),
            );
        }
        if (changes.spheres || changes.triangles) && !scene.bvh_primitives.is_empty() {
            context.queue.write_buffer(
                &buffers.bvh_primitives,
                0,
                bytemuck::cast_slice(&scene.bvh_primitives),
            );
        }
        let gpu_upload = upload_start.elapsed();
        let scene_update_upload = scene_update_start.elapsed();

        let dispatch_start = Instant::now();

        dispatch_compute_2d(ComputeDispatch {
            device: &context.device,
            queue: &context.queue,
            pipeline: &pipeline,
            bind_group: &bind_group,
            output: &buffers.output,
            readback: &buffers.readback,
            output_size: buffers.output_size,
            width: scene.params.width,
            height: scene.params.height,
        })?;
        let dispatch_wait = dispatch_start.elapsed();

        let readback_start = Instant::now();

        let pixels = readback_pixels(&context.device, &buffers.readback)?;
        let readback = readback_start.elapsed();

        let conversion_start = Instant::now();
        let image =
            pixels_to_rgba_image(&pixels, scene.params.width, scene.params.height, display)?;
        let conversion = conversion_start.elapsed();
        let render_time = frame_start.elapsed();
        let mut timings = GpuFrameTimings {
            animation_evaluation: evaluation_timings.animation_evaluation,
            physics_evaluation: evaluation_timings.physics_evaluation,
            geometry_update: evaluation_timings.geometry_update,
            light_rebuild: packing_timings.light_rebuild,
            bvh_rebuild: packing_timings.bvh_rebuild,
            gpu_upload,
            scene_update_upload,
            dispatch_wait,
            readback,
            conversion,
            output: Duration::ZERO,
            total: render_time,
        };
        let completed_samples = if progressive.is_some() {
            accumulated_samples
                .checked_add(scene.params.samples)
                .context("progressive accumulated sample count overflowed")?
        } else {
            scene.params.samples
        };
        let output_start = Instant::now();
        sink.deliver(CompletedFrame {
            index: frame,
            time_seconds,
            width: scene.params.width,
            height: scene.params.height,
            samples: scene.params.samples,
            accumulated_samples: completed_samples,
            max_bounces: scene.params.max_bounces,
            timings,
            render_time,
            image: &image,
            counters: &counters,
        })?;
        timings.output = output_start.elapsed();
        timings.total = frame_start.elapsed();
        sink.record_timings(frame, timings);

        tracing::debug!(
            frame.index = frame,
            animation_time_seconds = time_seconds as f64,
            samples = scene.params.samples,
            pixels = pixels.len(),
            animation_evaluation_ms = timings.animation_evaluation.as_secs_f64() * 1000.0,
            physics_evaluation_ms = timings.physics_evaluation.as_secs_f64() * 1000.0,
            geometry_update_ms = timings.geometry_update.as_secs_f64() * 1000.0,
            light_rebuilt = packing_timings.light_rebuilt,
            light_rebuild_ms = timings.light_rebuild.as_secs_f64() * 1000.0,
            bvh_rebuild_ms = timings.bvh_rebuild.as_secs_f64() * 1000.0,
            gpu_upload_ms = timings.gpu_upload.as_secs_f64() * 1000.0,
            scene_update_upload_ms = scene_update_upload.as_secs_f64() * 1000.0,
            dispatch_wait_ms = dispatch_wait.as_secs_f64() * 1000.0,
            readback_ms = readback.as_secs_f64() * 1000.0,
            conversion_ms = conversion.as_secs_f64() * 1000.0,
            output_ms = timings.output.as_secs_f64() * 1000.0,
            total_ms = timings.total.as_secs_f64() * 1000.0,
            "completed GPU frame"
        );

        accumulated_samples = completed_samples;
        if progressive.is_some_and(|config| accumulated_samples >= config.target_samples()) {
            tracing::info!(
                accumulated_samples,
                "progressive preview reached its sample target"
            );
            hold_completed_preview(animation, pacing, pacing_started_at, sink)?;
            break;
        }

        let next_frame = frame
            .checked_add(1)
            .context("render frame index exceeded the supported range")?;
        let has_next_frame = animation
            .frame_limit()
            .is_none_or(|frame_limit| next_frame < frame_limit);
        if !has_next_frame || !sink.should_continue() {
            break;
        }

        if pacing == FramePacing::RealTime {
            let fps = animation
                .fps()
                .context("real-time frame pacing requires an FPS")?;
            let next_deadline = pacing_started_at + frame_deadline_offset(next_frame, fps);
            if let Some(delay) = next_deadline.checked_duration_since(Instant::now()) {
                std::thread::sleep(delay);
            }
        }

        frame = next_frame;
    }
    tracing::info!(
        duration_ms = total_start.elapsed().as_secs_f64() * 1000.0,
        "completed GPU render"
    );
    Ok(GpuRenderSummary {
        adapter: context.info.clone(),
        setup_time,
    })
}

/// Keeps a converged progressive preview alive without dispatching more work.
fn hold_completed_preview(
    animation: AnimationConfig,
    pacing: FramePacing,
    pacing_started_at: Instant,
    sink: &dyn FrameSink,
) -> Result<()> {
    if pacing != FramePacing::RealTime || !sink.should_continue() {
        return Ok(());
    }

    const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(50);
    let deadline = match animation.frame_limit() {
        Some(frame_limit) => {
            let fps = animation
                .fps()
                .context("real-time frame pacing requires an FPS")?;
            Some(pacing_started_at + frame_deadline_offset(frame_limit, fps))
        }
        None => None,
    };

    while sink.should_continue() {
        let delay = match deadline {
            Some(deadline) => match deadline.checked_duration_since(Instant::now()) {
                Some(remaining) => remaining.min(CANCELLATION_POLL_INTERVAL),
                None => break,
            },
            None => CANCELLATION_POLL_INTERVAL,
        };
        std::thread::sleep(delay);
    }
    Ok(())
}

/// Converts a frame index and rate to its ideal elapsed deadline.
fn frame_deadline_offset(frame: u32, fps: u32) -> Duration {
    Duration::from_secs_f64(frame as f64 / fps as f64)
}

/// Keeps independent benchmark seeds fixed while normal frame seeds stay monotonic.
fn render_frame_seed(frame: u32, mode: FrameMode) -> u32 {
    match mode {
        FrameMode::Animated => frame,
        FrameMode::StaticIndependent => 0,
    }
}

/// Validates and clamps the next progressive batch to the remaining target.
fn progressive_batch(
    requested_samples: u32,
    accumulated_samples: u32,
    target_samples: u32,
) -> Result<Option<u32>> {
    ensure!(
        requested_samples > 0,
        "sample batch must be greater than zero"
    );
    ensure!(
        target_samples > 0,
        "sample target must be greater than zero"
    );
    ensure!(
        accumulated_samples <= target_samples,
        "accumulated samples exceed the progressive target"
    );
    if accumulated_samples == target_samples {
        return Ok(None);
    }
    Ok(Some(
        requested_samples.min(target_samples - accumulated_samples),
    ))
}

/// Rejects real-time schedules that cannot define frame deadlines.
fn validate_pacing(animation: AnimationConfig, pacing: FramePacing) -> Result<()> {
    if pacing == FramePacing::RealTime {
        ensure!(
            animation.fps().is_some(),
            "real-time frame pacing requires an FPS"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    #[derive(Default)]
    struct RecordingSink {
        frames: Vec<Vec<u8>>,
        counters: Vec<NamedCounterSnapshot>,
    }

    impl FrameSink for RecordingSink {
        fn deliver(&mut self, frame: CompletedFrame<'_>) -> Result<()> {
            let jpeg = crate::image_output::encode_rgba_image_to_jpeg(frame.image, 90)?;
            self.frames.push(jpeg);
            self.counters.push(frame.counters.clone());
            Ok(())
        }
    }

    #[test]
    fn frame_sink_accepts_encoded_completed_frame() {
        let image = pixels_to_rgba_image(
            &[[0.25, 0.5, 0.75, 1.0]],
            1,
            1,
            toaster_core::color::DisplaySettings::default(),
        )
        .unwrap();
        let mut sink = RecordingSink::default();
        let counters = NamedCounterSnapshot {
            loop_counts: [("hits".to_owned(), 2)].into(),
            session_counts: [("hits".to_owned(), 7)].into(),
        };

        sink.deliver(CompletedFrame {
            index: 0,
            time_seconds: 0.0,
            width: 1,
            height: 1,
            samples: 1,
            accumulated_samples: 1,
            max_bounces: 1,
            timings: GpuFrameTimings {
                total: Duration::from_millis(1),
                ..GpuFrameTimings::default()
            },
            render_time: Duration::from_millis(1),
            image: &image,
            counters: &counters,
        })
        .unwrap();

        assert_eq!(sink.frames.len(), 1);
        assert_eq!(sink.counters, vec![counters]);
        let decoded =
            image::load_from_memory_with_format(&sink.frames[0], image::ImageFormat::Jpeg).unwrap();
        assert_eq!(decoded.dimensions(), (1, 1));
    }

    #[test]
    fn real_time_deadlines_follow_requested_fps() {
        assert_eq!(frame_deadline_offset(1, 4), Duration::from_millis(250));
        assert_eq!(frame_deadline_offset(12, 12), Duration::from_secs(1));
    }

    #[test]
    fn looped_scene_time_does_not_wrap_render_rng_seed() {
        let schedule = AnimationConfig::indefinite(4)
            .unwrap()
            .with_loop_duration(2.0)
            .unwrap();

        assert_eq!(schedule.evaluation_request(0).time_seconds, 0.0);
        assert_eq!(schedule.evaluation_request(8).time_seconds, 0.0);
        assert_eq!(schedule.evaluation_request(8).loop_cycle, 1);
        assert_eq!(render_frame_seed(0, FrameMode::Animated), 0);
        assert_eq!(render_frame_seed(8, FrameMode::Animated), 8);
        assert_eq!(render_frame_seed(8, FrameMode::StaticIndependent), 0);
    }

    #[test]
    fn real_time_schedule_uses_every_sequential_frame_state() {
        let schedule = AnimationConfig::indefinite(4).unwrap();
        let requests = (0..4)
            .map(|frame| schedule.evaluation_request(frame).time_seconds)
            .collect::<Vec<_>>();

        assert_eq!(requests, vec![0.0, 0.25, 0.5, 0.75]);
    }

    #[test]
    fn real_time_pacing_rejects_a_schedule_without_fps_before_gpu_setup() {
        let mut sink = RecordingSink::default();
        let error = pollster::block_on(render_scene_gpu_animation_with_sink(
            Path::new("unused.json"),
            AnimationConfig::single_frame(),
            FramePacing::RealTime,
            &mut sink,
        ))
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("real-time frame pacing requires an FPS"));
    }

    #[test]
    fn progressive_config_and_batches_validate_and_clamp() {
        assert!(ProgressiveRenderConfig::new(0).is_err());
        assert_eq!(ProgressiveRenderConfig::new(5).unwrap().target_samples(), 5);
        assert_eq!(progressive_batch(2, 0, 5).unwrap(), Some(2));
        assert_eq!(progressive_batch(2, 2, 5).unwrap(), Some(2));
        assert_eq!(progressive_batch(2, 4, 5).unwrap(), Some(1));
        assert_eq!(progressive_batch(2, 5, 5).unwrap(), None);
        assert!(progressive_batch(0, 0, 5).is_err());
        assert!(progressive_batch(1, 6, 5).is_err());
    }

    #[test]
    fn benchmark_config_validates_frame_counts() {
        let config = GpuBenchmarkConfig::new(2, 5).unwrap();
        assert_eq!(config.warmup_frames(), 2);
        assert_eq!(config.measured_frames(), 5);
        assert!(GpuBenchmarkConfig::new(0, 0).is_err());
        assert!(GpuBenchmarkConfig::new(u32::MAX, 1).is_err());
    }

    #[test]
    fn completed_frame_total_timing_matches_compatibility_duration() {
        let total = Duration::from_millis(17);
        let timings = GpuFrameTimings {
            scene_update_upload: Duration::from_millis(1),
            dispatch_wait: Duration::from_millis(10),
            readback: Duration::from_millis(4),
            conversion: Duration::from_millis(2),
            total,
            ..GpuFrameTimings::default()
        };
        assert_eq!(timings.total, total);
    }

    #[test]
    fn progressive_render_rejects_animation_before_gpu_setup() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/006_rotating_cube.json");
        let scene = toaster_scene::load_scene(path).unwrap();
        let mut sink = RecordingSink::default();
        let error = pollster::block_on(render_gpu_progressive_with_sink(
            &scene,
            AnimationConfig::indefinite(12).unwrap(),
            FramePacing::RealTime,
            ProgressiveRenderConfig::new(16).unwrap(),
            &mut sink,
        ))
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("requires a scene without animation tracks"));
    }

    #[test]
    fn progressive_render_rejects_enabled_physics_before_gpu_setup() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/011_physics_rigid_bodies.json");
        let scene = toaster_scene::load_scene(path).unwrap();
        let mut sink = RecordingSink::default();
        let error = pollster::block_on(render_gpu_progressive_with_sink(
            &scene,
            AnimationConfig::indefinite(12).unwrap(),
            FramePacing::RealTime,
            ProgressiveRenderConfig::new(16).unwrap(),
            &mut sink,
        ))
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("does not support physics-enabled scenes"));
    }
}
