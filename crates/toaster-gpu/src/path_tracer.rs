//! Compute path-tracing pipeline.
use anyhow::Result;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::buffers::create_scene_gpu_buffers;
use crate::device::create_gpu_context;
use crate::dispatch::dispatch_compute_2d;
use crate::image_output::save_pixels_to_png;
use crate::pipeline::{
    create_pathtrace_bind_group, create_pathtrace_bind_group_layout, create_pipeline, load_shader,
};
use crate::readback::readback_pixels;
use crate::scene_upload::load_scene_gpu;

pub async fn render_scene_gpu(scene_path: &Path, out_path: &Path) -> Result<()> {
    let scene = load_scene_gpu(scene_path)?;
    let generation_started_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let total_start = Instant::now();

    println!(
        "Generation started at unix timestamp: {}",
        generation_started_at
    );

    println!(
        "GPU path tracer: {}x{}, samples={}, bounces={}, spheres={}, triangles={}, materials={}",
        scene.params.width,
        scene.params.height,
        scene.params.samples,
        scene.params.max_bounces,
        scene.params.sphere_count,
        scene.params.triangle_count,
        scene.params.material_count,
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
        "GPU dispatch + wait time: {:.3}s",
        dispatch_start.elapsed().as_secs_f64()
    );

    let readback_start = Instant::now();

    let pixels = readback_pixels(&context.device, &buffers.readback)?;
    println!("Read back {} pixels.", pixels.len());

    println!(
        "Readback time: {:.3}s",
        readback_start.elapsed().as_secs_f64()
    );

    let save_start = Instant::now();

    save_pixels_to_png(&pixels, scene.params.width, scene.params.height, out_path)?;

    println!("PNG save time: {:.3}s", save_start.elapsed().as_secs_f64());
    println!("Saved {}", out_path.display());

    println!(
        "Total generation time: {:.3}s",
        total_start.elapsed().as_secs_f64()
    );
    Ok(())
}
