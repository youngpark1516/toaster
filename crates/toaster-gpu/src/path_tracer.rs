//! Compute path-tracing pipeline.
use anyhow::Result;
use std::path::Path;

use crate::buffers::create_scene_gpu_buffers;
use crate::device::create_gpu_context;
use crate::dispatch::dispatch_compute_2d;
use crate::image_output::save_pixels_to_png;
use crate::pipeline::{
    create_pathtrace_bind_group, create_pathtrace_bind_group_layout, create_pipeline, load_shader,
};
use crate::readback::readback_pixels;
use crate::scene_upload::create_test_scene;

pub async fn render_scene_gpu(out_path: &Path) -> Result<()> {
    let scene = create_test_scene(800, 600);

    println!(
        "GPU path tracer: {}x{}, samples={}, bounces={}, spheres={}, materials={}",
        scene.params.width,
        scene.params.height,
        scene.params.samples,
        scene.params.max_bounces,
        scene.params.sphere_count,
        scene.params.material_count,
    );

    let context = create_gpu_context().await?;
    println!("GPU context created successfully.");

    let buffers = create_scene_gpu_buffers(&context.device, &scene);
    println!("Scene GPU buffers created.");

    let bind_group_layout = create_pathtrace_bind_group_layout(&context.device);
    println!("Pathtrace bind group layout created.");

    let bind_group = create_pathtrace_bind_group(&context.device, &bind_group_layout, &buffers);
    println!("Pathtrace bind group created.");

    let shader = load_shader(
        &context.device,
        "Pathtrace Spheres Shader",
        include_str!("../../../shaders/pathtrace_spheres.wgsl"),
    );
    println!("Pathtrace shader loaded.");

    let pipeline = create_pipeline(&context.device, &shader, &bind_group_layout);
    println!("Pathtrace compute pipeline created.");

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
    println!("Pathtrace dispatch completed.");

    let pixels = readback_pixels(&context.device, &buffers.readback)?;
    println!("Read back {} pixels.", pixels.len());

    save_pixels_to_png(&pixels, scene.params.width, scene.params.height, out_path)?;
    println!("Saved {}", out_path.display());

    Ok(())
}
