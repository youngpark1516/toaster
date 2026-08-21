//! Minimal GPU gradient renderer used to verify compute setup and readback.

use std::path::Path;

use anyhow::Result;

use crate::buffers::{create_pixel_buffers, RenderParams};
use crate::device::create_gpu_context;
use crate::dispatch::{dispatch_compute_2d, ComputeDispatch};
use crate::image_output::save_pixels_to_png;
use crate::pipeline::{create_bind_group, create_bind_group_layout, create_pipeline, load_shader};
use crate::readback::readback_pixels;

/// Returns the fixed settings used by the diagnostic gradient command.
fn default_render_params() -> RenderParams {
    RenderParams {
        width: 800,
        height: 600,
        samples: 16,
        max_bounces: 8,
    }
}

/// Renders the diagnostic gradient and writes it as a PNG at `out_path`.
///
/// This initializes a fresh GPU context and returns an error if adapter setup,
/// dispatch, readback, or image encoding fails.
pub async fn render_gradient(out_path: &Path) -> Result<()> {
    let params = default_render_params();

    tracing::info!(
        width = params.width,
        height = params.height,
        samples = params.samples,
        max_bounces = params.max_bounces,
        "rendering GPU gradient"
    );

    let context = create_gpu_context().await?;
    tracing::debug!("created GPU context");

    let buffers = create_pixel_buffers(&context.device, &context.queue, &params);
    tracing::debug!(
        output_bytes = buffers.output_size,
        readback_bytes = buffers.output_size,
        params_bytes = buffers.params_size,
        "created gradient buffers"
    );
    let bind_group_layout = create_bind_group_layout(&context.device);
    tracing::debug!("created gradient bind group layout");

    let bind_group = create_bind_group(&context.device, &bind_group_layout, &buffers);
    tracing::debug!("created gradient bind group");

    let shader = load_shader(
        &context.device,
        "Gradient Shader",
        include_str!("../../../shaders/gpu_gradient.wgsl"),
    );
    tracing::debug!("loaded gradient shader");

    let pipeline = create_pipeline(&context.device, &shader, &bind_group_layout);
    tracing::debug!("created gradient compute pipeline");

    dispatch_compute_2d(ComputeDispatch {
        device: &context.device,
        queue: &context.queue,
        pipeline: &pipeline,
        bind_group: &bind_group,
        output: &buffers.output,
        readback: &buffers.readback,
        output_size: buffers.output_size,
        width: params.width,
        height: params.height,
    })?;
    tracing::debug!("completed gradient compute dispatch");

    let pixels = readback_pixels(&context.device, &buffers.readback)?;
    tracing::debug!(pixels = pixels.len(), "read back gradient pixels");

    save_pixels_to_png(
        &pixels,
        params.width,
        params.height,
        toaster_core::color::DisplaySettings::default(),
        out_path,
    )?;
    tracing::info!(path = %out_path.display(), "wrote gradient PNG");

    Ok(())
}
