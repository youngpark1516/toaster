//! Compute path-tracing pipeline.
use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::animation::{frame_output_path, AnimationConfig};
use crate::buffers::create_scene_gpu_buffers;
use crate::device::create_gpu_context;
use crate::dispatch::dispatch_compute_2d;
use crate::image_output::save_pixels_to_png;
use crate::pipeline::{
    create_pathtrace_bind_group, create_pathtrace_bind_group_layout, create_pipeline, load_shader,
};
use crate::readback::readback_pixels;
use crate::scene_upload::scene_to_gpu;
use crate::video_output::FfmpegVideoWriter;

enum OutputRequest<'a> {
    PngSequence(&'a Path),
    Video { path: &'a Path, fps: u32 },
}

enum FrameOutput<'a> {
    PngSequence(&'a Path),
    Video(FfmpegVideoWriter),
}

impl<'a> FrameOutput<'a> {
    fn start(request: OutputRequest<'a>, width: u32, height: u32) -> Result<Self> {
        match request {
            OutputRequest::PngSequence(path) => Ok(Self::PngSequence(path)),
            OutputRequest::Video { path, fps } => Ok(Self::Video(FfmpegVideoWriter::start(
                path, width, height, fps,
            )?)),
        }
    }

    fn write_frame(
        &mut self,
        pixels: &[[f32; 4]],
        width: u32,
        height: u32,
        frame: u32,
        frame_count: u32,
    ) -> Result<Option<PathBuf>> {
        match self {
            Self::PngSequence(out_path) => {
                let frame_path = frame_output_path(out_path, frame, frame_count);
                save_pixels_to_png(pixels, width, height, &frame_path)?;
                Ok(Some(frame_path))
            }
            Self::Video(writer) => {
                writer.write_frame(pixels, width, height)?;
                Ok(None)
            }
        }
    }

    fn finish(self) -> Result<()> {
        match self {
            Self::PngSequence(_) => Ok(()),
            Self::Video(writer) => writer.finish(),
        }
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
    render_scene_gpu_animation_inner(scene_path, animation, OutputRequest::PngSequence(out_path))
        .await
}

pub async fn render_scene_gpu_video(
    scene_path: &Path,
    video_path: &Path,
    animation: AnimationConfig,
) -> Result<()> {
    let Some(fps) = animation.fps() else {
        bail!("video export requires an animated render with --fps");
    };
    render_scene_gpu_animation_inner(
        scene_path,
        animation,
        OutputRequest::Video {
            path: video_path,
            fps,
        },
    )
    .await
}

async fn render_scene_gpu_animation_inner(
    scene_path: &Path,
    animation: AnimationConfig,
    output_request: OutputRequest<'_>,
) -> Result<()> {
    let source_scene = toaster_scene::load_scene(scene_path)?;
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

    let mut frame_output =
        FrameOutput::start(output_request, scene.params.width, scene.params.height)?;

    println!(
        "GPU setup time: {:.3}s",
        setup_start.elapsed().as_secs_f64()
    );

    let frame_count = animation.frame_count();

    let render_result = (|| -> Result<()> {
        for frame in 0..frame_count {
            let frame_start = Instant::now();
            let evaluated = source_scene.evaluate_at(animation.time_for_frame(frame))?;
            scene = scene_to_gpu(&evaluated)?;
            scene.params.frame_index = frame;

            context
                .queue
                .write_buffer(&buffers.camera, 0, bytemuck::bytes_of(&scene.camera));
            context
                .queue
                .write_buffer(&buffers.params, 0, bytemuck::bytes_of(&scene.params));
            if !scene.spheres.is_empty() {
                context.queue.write_buffer(
                    &buffers.spheres,
                    0,
                    bytemuck::cast_slice(&scene.spheres),
                );
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

            let output_start = Instant::now();
            let frame_path = frame_output.write_frame(
                &pixels,
                scene.params.width,
                scene.params.height,
                frame,
                frame_count,
            )?;

            println!(
                "Frame {} output time: {:.3}s",
                frame,
                output_start.elapsed().as_secs_f64()
            );

            match frame_path {
                Some(frame_path) => println!(
                    "Saved frame {} to {} in {:.3}s",
                    frame,
                    frame_path.display(),
                    frame_start.elapsed().as_secs_f64()
                ),
                None => println!(
                    "Submitted video frame {} in {:.3}s",
                    frame,
                    frame_start.elapsed().as_secs_f64()
                ),
            }
        }
        Ok(())
    })();
    let finish_result = frame_output.finish();
    if let Err(render_error) = render_result {
        return match finish_result {
            Ok(()) => Err(render_error),
            Err(finish_error) => Err(anyhow!(
                "{render_error:#}; output finalization also failed: {finish_error:#}"
            )),
        };
    }
    finish_result?;
    println!(
        "Total generation time: {:.3}s",
        total_start.elapsed().as_secs_f64()
    );
    Ok(())
}
