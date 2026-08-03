//! Compute path-tracing pipeline.
use anyhow::{anyhow, ensure, Context, Result};
use image::RgbaImage;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::animation::{frame_output_path, AnimationConfig};
use crate::buffers::create_scene_gpu_buffers;
use crate::device::{create_gpu_context, GpuAdapterInfo};
use crate::dispatch::{dispatch_compute_2d, ComputeDispatch};
use crate::image_output::{pixels_to_rgba_image, save_rgba_image_to_png};
use crate::pipeline::{
    create_pathtrace_bind_group, create_pathtrace_bind_group_layout, create_pipeline, load_shader,
};
use crate::readback::readback_pixels;
use crate::scene_upload::{scene_to_gpu, scene_to_gpu_frame};
use crate::video_output::FfmpegVideoWriter;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramePacing {
    Unpaced,
    RealTime,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GpuFrameTimings {
    pub scene_update_upload: Duration,
    pub dispatch_wait: Duration,
    pub readback: Duration,
    pub conversion: Duration,
    pub total: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GpuBenchmarkConfig {
    warmup_frames: u32,
    measured_frames: u32,
}

impl GpuBenchmarkConfig {
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

    pub fn warmup_frames(self) -> u32 {
        self.warmup_frames
    }

    pub fn measured_frames(self) -> u32 {
        self.measured_frames
    }
}

#[derive(Clone, Debug)]
pub struct GpuBenchmarkResult {
    pub adapter: GpuAdapterInfo,
    pub setup_time: Duration,
    pub frames: Vec<GpuFrameTimings>,
    pub final_image: RgbaImage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgressiveRenderConfig {
    target_samples: u32,
}

impl ProgressiveRenderConfig {
    pub fn new(target_samples: u32) -> Result<Self> {
        ensure!(
            target_samples > 0,
            "progressive target samples must be greater than zero"
        );
        Ok(Self { target_samples })
    }

    pub fn target_samples(self) -> u32 {
        self.target_samples
    }
}

pub struct CompletedFrame<'a> {
    pub index: u32,
    pub time_seconds: f32,
    pub width: u32,
    pub height: u32,
    /// Samples rendered by this dispatch.
    pub samples: u32,
    /// Total samples represented by `image` after this dispatch.
    pub accumulated_samples: u32,
    pub max_bounces: u32,
    pub timings: GpuFrameTimings,
    /// Total time through RGBA conversion, retained for compatibility.
    pub render_time: Duration,
    pub image: &'a RgbaImage,
}

pub trait FrameSink {
    fn deliver(&mut self, frame: CompletedFrame<'_>) -> Result<()>;

    fn should_continue(&self) -> bool {
        true
    }

    fn samples_for_frame(&self) -> Option<u32> {
        None
    }
}

struct PngFrameSink<'a> {
    out_path: &'a Path,
    frame_count: u32,
}

impl FrameSink for PngFrameSink<'_> {
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

struct VideoFrameSink {
    writer: Option<FfmpegVideoWriter>,
}

impl VideoFrameSink {
    fn finish(mut self) -> Result<()> {
        self.writer
            .take()
            .context("video writer is unavailable")?
            .finish()
    }
}

impl FrameSink for VideoFrameSink {
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

struct BenchmarkFrameSink {
    warmup_frames: u32,
    timings: Vec<GpuFrameTimings>,
    final_image: Option<RgbaImage>,
}

impl FrameSink for BenchmarkFrameSink {
    fn deliver(&mut self, frame: CompletedFrame<'_>) -> Result<()> {
        if frame.index >= self.warmup_frames {
            self.timings.push(frame.timings);
            self.final_image = Some(frame.image.clone());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrameMode {
    Animated,
    StaticIndependent,
}

struct GpuRenderSummary {
    adapter: GpuAdapterInfo,
    setup_time: Duration,
}

pub async fn render_scene_gpu(scene_path: &Path, out_path: &Path) -> Result<()> {
    render_scene_gpu_animation(scene_path, out_path, AnimationConfig::single_frame()).await
}

pub async fn render_scene_gpu_animation(
    scene_path: &Path,
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
    render_scene_gpu_animation_with_sink(scene_path, animation, FramePacing::Unpaced, &mut sink)
        .await
}

pub async fn render_scene_gpu_video(
    scene_path: &Path,
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

    let source_scene = toaster_scene::load_scene(scene_path)?;
    let mut sink = VideoFrameSink {
        writer: Some(FfmpegVideoWriter::start(
            video_path,
            source_scene.render.width,
            source_scene.render.height,
            fps,
        )?),
    };
    let render_result =
        render_gpu_animation_with_sink(&source_scene, animation, FramePacing::Unpaced, &mut sink)
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

pub async fn render_scene_gpu_animation_with_sink(
    scene_path: &Path,
    animation: AnimationConfig,
    pacing: FramePacing,
    sink: &mut dyn FrameSink,
) -> Result<()> {
    validate_pacing(animation, pacing)?;
    let source_scene = toaster_scene::load_scene(scene_path)?;
    render_gpu_animation_with_sink(&source_scene, animation, pacing, sink).await
}

pub async fn render_gpu_animation_with_sink(
    source_scene: &toaster_scene::Scene,
    animation: AnimationConfig,
    pacing: FramePacing,
    sink: &mut dyn FrameSink,
) -> Result<()> {
    render_gpu_with_sink(
        source_scene,
        animation,
        pacing,
        None,
        FrameMode::Animated,
        sink,
    )
    .await
    .map(|_| ())
}

pub async fn render_gpu_progressive_with_sink(
    source_scene: &toaster_scene::Scene,
    schedule: AnimationConfig,
    pacing: FramePacing,
    config: ProgressiveRenderConfig,
    sink: &mut dyn FrameSink,
) -> Result<()> {
    ensure!(
        source_scene.animation.tracks.is_empty(),
        "progressive preview requires a scene without animation tracks"
    );
    ensure!(
        schedule.loop_duration().is_none(),
        "progressive preview does not support an animation loop duration"
    );
    render_gpu_with_sink(
        source_scene,
        schedule,
        pacing,
        Some(config),
        FrameMode::Animated,
        sink,
    )
    .await
    .map(|_| ())
}

pub async fn benchmark_gpu_scene(
    source_scene: &toaster_scene::Scene,
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
        source_scene,
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

async fn render_gpu_with_sink(
    source_scene: &toaster_scene::Scene,
    animation: AnimationConfig,
    pacing: FramePacing,
    progressive: Option<ProgressiveRenderConfig>,
    frame_mode: FrameMode,
    sink: &mut dyn FrameSink,
) -> Result<GpuRenderSummary> {
    validate_pacing(animation, pacing)?;

    let initial_scene = source_scene.evaluate_at(0.0)?;
    let mut scene = scene_to_gpu(&initial_scene)?;
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
        let time_seconds = if progressive.is_some() || frame_mode == FrameMode::StaticIndependent {
            0.0
        } else {
            animation.time_for_frame(frame)
        };
        scene = if progressive.is_some() || frame_mode == FrameMode::StaticIndependent {
            scene_to_gpu_frame(&initial_scene)?
        } else {
            let evaluated = source_scene.evaluate_at(time_seconds)?;
            scene_to_gpu_frame(&evaluated)?
        };
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
        scene.params.frame_index = if frame_mode == FrameMode::StaticIndependent {
            0
        } else {
            frame
        };
        scene.params.accumulated_samples = if progressive.is_some() {
            accumulated_samples
        } else {
            0
        };

        context
            .queue
            .write_buffer(&buffers.camera, 0, bytemuck::bytes_of(&scene.camera));
        context
            .queue
            .write_buffer(&buffers.params, 0, bytemuck::bytes_of(&scene.params));
        if !scene.spheres.is_empty() {
            context
                .queue
                .write_buffer(&buffers.spheres, 0, bytemuck::cast_slice(&scene.spheres));
        }
        if !scene.triangles.is_empty() {
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
        if !scene.lights.is_empty() {
            context
                .queue
                .write_buffer(&buffers.lights, 0, bytemuck::cast_slice(&scene.lights));
        }
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
        let image = pixels_to_rgba_image(&pixels, scene.params.width, scene.params.height)?;
        let conversion = conversion_start.elapsed();
        let render_time = frame_start.elapsed();
        let timings = GpuFrameTimings {
            scene_update_upload,
            dispatch_wait,
            readback,
            conversion,
            total: render_time,
        };
        let completed_samples = if progressive.is_some() {
            accumulated_samples
                .checked_add(scene.params.samples)
                .context("progressive accumulated sample count overflowed")?
        } else {
            scene.params.samples
        };
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
        })?;

        tracing::debug!(
            frame.index = frame,
            animation_time_seconds = time_seconds as f64,
            samples = scene.params.samples,
            pixels = pixels.len(),
            scene_update_upload_ms = scene_update_upload.as_secs_f64() * 1000.0,
            dispatch_wait_ms = dispatch_wait.as_secs_f64() * 1000.0,
            readback_ms = readback.as_secs_f64() * 1000.0,
            conversion_ms = conversion.as_secs_f64() * 1000.0,
            total_ms = render_time.as_secs_f64() * 1000.0,
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

fn frame_deadline_offset(frame: u32, fps: u32) -> Duration {
    Duration::from_secs_f64(frame as f64 / fps as f64)
}

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
    }

    impl FrameSink for RecordingSink {
        fn deliver(&mut self, frame: CompletedFrame<'_>) -> Result<()> {
            let jpeg = crate::image_output::encode_rgba_image_to_jpeg(frame.image, 90)?;
            self.frames.push(jpeg);
            Ok(())
        }
    }

    #[test]
    fn frame_sink_accepts_encoded_completed_frame() {
        let image = pixels_to_rgba_image(&[[0.25, 0.5, 0.75, 1.0]], 1, 1).unwrap();
        let mut sink = RecordingSink::default();

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
        })
        .unwrap();

        assert_eq!(sink.frames.len(), 1);
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
}
