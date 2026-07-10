//! Compute path-tracing pipeline.
use anyhow::Result;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use glam::Vec3;

use crate::buffers::create_scene_gpu_buffers;
use crate::device::create_gpu_context;
use crate::dispatch::dispatch_compute_2d;
use crate::image_output::save_pixels_to_png;
use crate::pipeline::{
    create_pathtrace_bind_group, create_pathtrace_bind_group_layout, create_pipeline, load_shader,
};
use crate::animation::{AnimationConfig, frame_output_path};
use crate::readback::readback_pixels;
use crate::scene_upload::{load_scene_gpu, make_camera};
use crate::gpu_types::{GpuCamera};

pub async fn render_scene_gpu(scene_path: &Path, out_path: &Path) -> Result<()> {
    render_scene_gpu_animation(scene_path, out_path, AnimationConfig::single_frame()).await
}

pub async fn render_scene_gpu_animation(
    scene_path: &Path,
    out_path: &Path,
    animation: AnimationConfig,
) -> Result<()> {
    let source_scene = toaster_scene::load_scene(scene_path)?;
    let mut scene = crate::scene_upload::scene_to_gpu(&source_scene)?;
    let base_camera = scene.camera;
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

    let frame_count = animation.frame_count();

    for frame in 0..frame_count {
        let progress = if frame_count <= 1 {
            0.0
        } else {
            frame as f32 / (frame_count - 1) as f32
        };

        let camera_x = -1.0 + 2.0 * progress;

        scene.camera = translate_camera(base_camera, [camera_x, 0.0, 0.0]);

        context.queue.write_buffer(
            &buffers.camera,
            0,
            bytemuck::bytes_of(&scene.camera),
        );

        scene.params.frame_index = frame;

        context.queue.write_buffer(
            &buffers.params,
            0,
            bytemuck::bytes_of(&scene.params),
        );

        let frame_start = Instant::now();

        if let Some(orbit_degrees) = animation.orbit_degrees {
            scene.camera = orbit_camera_for_frame(
                &source_scene,
                frame,
                frame_count,
                orbit_degrees,
            );

            context.queue.write_buffer(
                &buffers.camera,
                0,
                bytemuck::bytes_of(&scene.camera),
            );
        }

        scene.params.frame_index = frame;

        context.queue.write_buffer(
            &buffers.params,
            0,
            bytemuck::bytes_of(&scene.params),
        );

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

        let save_start = Instant::now();

        let frame_path = frame_output_path(out_path, frame, frame_count);

        save_pixels_to_png(
            &pixels,
            scene.params.width,
            scene.params.height,
            &frame_path,
        )?;

        println!(
            "Frame {} PNG save time: {:.3}s",
            frame,
            save_start.elapsed().as_secs_f64()
        );

        println!(
            "Saved frame {} to {} in {:.3}s",
            frame,
            frame_path.display(),
            frame_start.elapsed().as_secs_f64()
        );
    }
    println!(
        "Total generation time: {:.3}s",
        total_start.elapsed().as_secs_f64()
    );
    Ok(())
}

fn translate_camera(camera: GpuCamera, offset: [f32; 3]) -> GpuCamera {
    let add_offset = |v: [f32; 4]| -> [f32; 4] {
        [
            v[0] + offset[0],
            v[1] + offset[1],
            v[2] + offset[2],
            v[3],
        ]
    };

    GpuCamera {
        origin: add_offset(camera.origin),
        lower_left_corner: add_offset(camera.lower_left_corner),
        horizontal: camera.horizontal,
        vertical: camera.vertical,
    }
}

fn orbit_camera_for_frame(
    source_scene: &toaster_scene::Scene,
    frame: u32,
    frame_count: u32,
    orbit_degrees: f32,
) -> crate::gpu_types::GpuCamera {
    let camera = &source_scene.camera;

    let look_at = camera.look_at;
    let offset = camera.position - look_at;

    let radius_xz = (offset.x * offset.x + offset.z * offset.z)
        .sqrt()
        .max(0.001);

    let base_angle = offset.z.atan2(offset.x);

    let progress = if frame_count <= 1 {
        0.0
    } else {
        frame as f32 / (frame_count - 1) as f32
    };

    let angle = base_angle + orbit_degrees.to_radians() * progress;

    let position = Vec3::new(
        look_at.x + radius_xz * angle.cos(),
        look_at.y + offset.y,
        look_at.z + radius_xz * angle.sin(),
    );

    let aspect = source_scene.render.width as f32 / source_scene.render.height as f32;

    crate::scene_upload::make_camera(
        position,
        look_at,
        camera.up,
        camera.fov_degrees,
        aspect,
    )
}