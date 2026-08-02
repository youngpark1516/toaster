use anyhow::Result;

pub struct ComputeDispatch<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub pipeline: &'a wgpu::ComputePipeline,
    pub bind_group: &'a wgpu::BindGroup,
    pub output: &'a wgpu::Buffer,
    pub readback: &'a wgpu::Buffer,
    pub output_size: u64,
    pub width: u32,
    pub height: u32,
}

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
