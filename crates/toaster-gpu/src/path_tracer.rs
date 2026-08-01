//! Compute path-tracing pipeline.
use anyhow::{ensure, Context, Result};
use image::RgbaImage;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::animation::{frame_output_path, AnimationConfig};
use crate::buffers::create_scene_gpu_buffers;
use crate::device::create_gpu_context;
use crate::dispatch::dispatch_compute_2d;
use crate::image_output::{pixels_to_rgba_image, save_rgba_image_to_png};
use crate::pipeline::{
    create_pathtrace_bind_group, create_pathtrace_bind_group_layout, create_pipeline, load_shader,
};
use crate::readback::readback_pixels;
use crate::scene_upload::scene_to_gpu;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramePacing {
    Unpaced,
    RealTime,
}

pub struct CompletedFrame<'a> {
    pub index: u32,
    pub time_seconds: f32,
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub max_bounces: u32,
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

        println!(
            "Frame {} PNG save time: {:.3}s",
            frame.index,
            save_start.elapsed().as_secs_f64()
        );
        println!("Saved frame {} to {}", frame.index, frame_path.display());
        Ok(())
    }
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
    validate_pacing(animation, pacing)?;

    let initial_scene = source_scene.evaluate_at(0.0)?;
    let mut scene = scene_to_gpu(&initial_scene)?;
    let generation_started_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let total_start = Instant::now();

    println!(
        "Generation started at unix timestamp: {}",
        generation_started_at
    );

    println!(
        "GPU path tracer: {}x{}, samples={}, bounces={}, spheres={}, triangles={}, materials={}, lights={}",
        scene.params.width,
        scene.params.height,
        scene.params.samples,
        scene.params.max_bounces,
        scene.params.sphere_count,
        scene.params.triangle_count,
        scene.params.material_count,
        scene.params.light_count,
    );

    let setup_start = Instant::now();

    let context = create_gpu_context().await?;
    println!("GPU context created successfully.");

    let buffers = create_scene_gpu_buffers(&context.device, &scene);
    println!("Scene GPU buffers created.");

    let bind_group_layout = create_pathtrace_bind_group_layout(&context.device);
    let bind_group = create_pathtrace_bind_group(&context.device, &bind_group_layout, &buffers);

    let shader = load_shader(
        &context.device,
        "Pathtrace Shader",
        include_str!("../../../shaders/pathtrace_spheres.wgsl"),
    );

    let pipeline = create_pipeline(&context.device, &shader, &bind_group_layout);

    println!(
        "GPU setup time: {:.3}s",
        setup_start.elapsed().as_secs_f64()
    );

    let pacing_started_at = Instant::now();
    let mut frame = 0_u32;

    while animation
        .frame_limit()
        .map_or(true, |frame_limit| frame < frame_limit)
    {
        if !sink.should_continue() {
            println!("GPU render stopped before frame {frame}.");
            break;
        }

        let frame_start = Instant::now();
        let time_seconds = animation.time_for_frame(frame);
        let evaluated = source_scene.evaluate_at(time_seconds)?;
        scene = scene_to_gpu(&evaluated)?;
        if let Some(samples) = sink.samples_for_frame() {
            ensure!(
                samples > 0,
                "frame sink sample count must be greater than zero"
            );
            scene.params.samples = samples;
        }
        scene.params.frame_index = frame;

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
        }
        if !scene.lights.is_empty() {
            context
                .queue
                .write_buffer(&buffers.lights, 0, bytemuck::cast_slice(&scene.lights));
        }

        let dispatch_start = Instant::now();

        dispatch_compute_2d(
            &context.device,
            &context.queue,
            &pipeline,
            &bind_group,
            &buffers.output,
            &buffers.readback,
            buffers.output_size,
            scene.params.width,
            scene.params.height,
        )?;

        println!(
            "Frame {} GPU dispatch + wait time: {:.3}s",
            frame,
            dispatch_start.elapsed().as_secs_f64()
        );

        let readback_start = Instant::now();

        let pixels = readback_pixels(&context.device, &buffers.readback)?;
        println!("Frame {} read back {} pixels.", frame, pixels.len());

        println!(
            "Frame {} readback time: {:.3}s",
            frame,
            readback_start.elapsed().as_secs_f64()
        );

        let image = pixels_to_rgba_image(&pixels, scene.params.width, scene.params.height)?;
        let render_time = frame_start.elapsed();
        sink.deliver(CompletedFrame {
            index: frame,
            time_seconds,
            width: scene.params.width,
            height: scene.params.height,
            samples: scene.params.samples,
            max_bounces: scene.params.max_bounces,
            render_time,
            image: &image,
        })?;

        println!(
            "Completed frame {} in {:.3}s",
            frame,
            frame_start.elapsed().as_secs_f64()
        );

        let next_frame = frame
            .checked_add(1)
            .context("render frame index exceeded the supported range")?;
        let has_next_frame = animation
            .frame_limit()
            .map_or(true, |frame_limit| next_frame < frame_limit);
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
    println!(
        "Total generation time: {:.3}s",
        total_start.elapsed().as_secs_f64()
    );
    Ok(())
}

fn frame_deadline_offset(frame: u32, fps: u32) -> Duration {
    Duration::from_secs_f64(frame as f64 / fps as f64)
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
            max_bounces: 1,
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
}
