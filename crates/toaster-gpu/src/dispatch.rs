use anyhow::Result;

pub fn dispatch_compute_2d(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::ComputePipeline,
    bind_group: &wgpu::BindGroup,
    output: &wgpu::Buffer,
    readback: &wgpu::Buffer,
    output_size: u64,
    width: u32,
    height: u32,
) -> Result<()> {
    let workgroups_x = width.div_ceil(8);
    let workgroups_y = height.div_ceil(8);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Command Encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Compute Pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
    }

    encoder.copy_buffer_to_buffer(output, 0, readback, 0, output_size);

    queue.submit(Some(encoder.finish()));

    device.poll(wgpu::PollType::Wait)?;

    Ok(())
}
