use std::path::Path;

use anyhow::Result;

use crate::buffers::{create_pixel_buffers, RenderParams};
use crate::device::create_gpu_context;
use crate::dispatch::dispatch_compute_2d;
use crate::image_output::save_pixels_to_png;
use crate::pipeline::{create_bind_group, create_bind_group_layout, create_pipeline, load_shader};
use crate::readback::readback_pixels;

fn default_render_params() -> RenderParams {
    RenderParams {
        width: 800,
        height: 600,
        samples: 16,
        max_bounces: 8,
    }
}

pub async fn render_gradient(out_path: &Path) -> Result<()> {
    let params = default_render_params();

    println!(
        "Rendering gradient with width={}, height={}, samples={}, max_bounces={}",
        params.width, params.height, params.samples, params.max_bounces
    );

    let context = create_gpu_context().await?;
    println!("GPU context created successfully.");

    let buffers = create_pixel_buffers(&context.device, &context.queue, &params);
    println!(
        "output buffer {} bytes, readback buffer size {} bytes, params buffer size {} bytes",
        buffers.output_size, buffers.output_size, buffers.params_size
    );
    let bind_group_layout = create_bind_group_layout(&context.device);
    println!("Gradient bind group layout created.");

    let bind_group = create_bind_group(&context.device, &bind_group_layout, &buffers);
    println!("Gradient bind group created.");

    let shader = load_shader(&context.device);
    println!("Gradient shader loaded.");

    let pipeline = create_pipeline(&context.device, &shader, &bind_group_layout);
    println!("Compute pipeline created.");

    let dispatch_result = dispatch_compute_2d(
        &context.device,
        &context.queue,
        &pipeline,
        &bind_group,
        &buffers,
        &params,
    )?;
    println!("Compute dispatch completed successfully.");

    let pixels = readback_pixels(&context.device, &buffers.readback)?;
    println!("Read back {} pixels.", pixels.len());

    save_pixels_to_png(&pixels, params.width, params.height, out_path)?;

    Ok(())
}
