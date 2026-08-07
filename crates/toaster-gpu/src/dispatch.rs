//! Compute-command encoding, output copying, submission, and synchronous wait.

use anyhow::Result;

/// Borrowed resources required for one two-dimensional compute dispatch.
pub struct ComputeDispatch<'a> {
    /// Device used to create commands and wait for completion.
    pub device: &'a wgpu::Device,
    /// Queue receiving the encoded command buffer.
    pub queue: &'a wgpu::Queue,
    /// Compute pipeline with an 8×8 workgroup entry point.
    pub pipeline: &'a wgpu::ComputePipeline,
    /// Bind group matching the pipeline layout.
    pub bind_group: &'a wgpu::BindGroup,
    /// Shader-written storage buffer.
    pub output: &'a wgpu::Buffer,
    /// CPU-mappable destination for a post-dispatch copy.
    pub readback: &'a wgpu::Buffer,
    /// Bytes copied from output into readback.
    pub output_size: u64,
    /// Logical output width.
    pub width: u32,
    /// Logical output height.
    pub height: u32,
}

/// Dispatches enough 8×8 workgroups to cover the image, copies output, and waits.
pub fn dispatch_compute_2d(dispatch: ComputeDispatch<'_>) -> Result<()> {
    let workgroups_x = dispatch.width.div_ceil(8);
    let workgroups_y = dispatch.height.div_ceil(8);

    let mut encoder = dispatch
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Command Encoder"),
        });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Compute Pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(dispatch.pipeline);
        pass.set_bind_group(0, dispatch.bind_group, &[]);
        pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
    }

    encoder.copy_buffer_to_buffer(
        dispatch.output,
        0,
        dispatch.readback,
        0,
        dispatch.output_size,
    );

    dispatch.queue.submit(Some(encoder.finish()));

    dispatch.device.poll(wgpu::PollType::Wait)?;

    Ok(())
}
