use std::{ops::Div, path::Path};

use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use wgpu::wgc::{device::queue, pipeline};

use image::{ImageBuffer, Rgb};

use crate::device::create_gpu_context;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RenderParams {
    width: u32,
    height: u32,
    samples: u32,
    max_bounces: u32,
}

struct GradientBuffers {
    output: wgpu::Buffer,
    readback: wgpu::Buffer,
    params: wgpu::Buffer,
    output_size: u64,
    params_size: u64,
}

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

    let buffers = create_gradient_pixel_buffers(&context.device, &context.queue, &params);
    println!(
        "output buffer {} bytes, readback buffer size {} bytes, params buffer size {} bytes",
        buffers.output_size, buffers.output_size, buffers.params_size
    );
    let bind_group_layout = create_gradient_bind_group_layout(&context.device);
    println!("Gradient bind group layout created.");

    let bind_group = create_gradient_bind_group(&context.device, &bind_group_layout, &buffers);
    println!("Gradient bind group created.");

    let shader = load_gradient_shader(&context.device);
    println!("Gradient shader loaded.");

    let pipeline = create_gradient_pipeline(&context.device, &shader, &bind_group_layout);
    println!("Compute pipeline created.");

    let dispatch_result = dispatch_gradient(
        &context.device,
        &context.queue,
        &pipeline,
        &bind_group,
        &buffers,
        &params,
    );
    match dispatch_result {
        Ok(_) => println!("Gradient dispatched successfully."),
        Err(e) => println!("Error dispatching gradient: {:?}", e),
    }

    let pixels = readback_pixels(&context.device, &buffers.readback)?;
    println!("Read back {} pixels.", pixels.len());

    save_pixels_to_png(pixels, params.width, params.height, out_path);

    Ok(())
}

fn create_gradient_pixel_buffers(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    params: &RenderParams,
) -> GradientBuffers {
    let pixel_count = params.width as u64 * params.height as u64;
    let output_size = pixel_count * std::mem::size_of::<[f32; 4]>() as u64;
    let params_size = std::mem::size_of::<RenderParams>() as u64;

    let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Output Buffer"),
        size: output_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Readback Buffer"),
        size: output_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Params Buffer"),
        size: params_size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&params_buffer, 0, bytemuck::bytes_of(params));

    GradientBuffers {
        output: output_buffer,
        readback: readback_buffer,
        params: params_buffer,
        output_size,
        params_size,
    }
}

fn create_gradient_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Bind Group Layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn create_gradient_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: &GradientBuffers,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.output.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.params.as_entire_binding(),
            },
        ],
    })
}

fn load_gradient_shader(device: &wgpu::Device) -> wgpu::ShaderModule {
    let shader_source = include_str!("../../../shaders/gpu_gradient.wgsl");
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Gradient Shader"),
        source: wgpu::ShaderSource::Wgsl(shader_source.into()),
    })
}

fn create_gradient_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Pipeline Layout"),
        bind_group_layouts: &[bind_group_layout],
        push_constant_ranges: &[],
    });

    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Compute Pipeline"),
        layout: Some(&pipeline_layout),
        module: shader,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

fn dispatch_gradient(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::ComputePipeline,
    bind_group: &wgpu::BindGroup,
    buffers: &GradientBuffers,
    params: &RenderParams,
) -> Result<()> {
    let workgroups_x = params.width.div_ceil(8);
    let workgroups_y = params.height.div_ceil(8);

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

    encoder.copy_buffer_to_buffer(
        &buffers.output,
        0,
        &buffers.readback,
        0,
        buffers.output_size,
    );

    queue.submit(Some(encoder.finish()));

    device.poll(wgpu::PollType::Wait)?;

    Ok(())
}

fn readback_pixels(device: &wgpu::Device, readback: &wgpu::Buffer) -> Result<Vec<[f32; 4]>> {
    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).expect("Failed to send map result");
    });

    device.poll(wgpu::PollType::Wait);

    receiver.recv().expect("Failed to receive map result")?;

    let data = slice.get_mapped_range();
    let pixels: Vec<[f32; 4]> = bytemuck::cast_slice(&data).to_vec();
    drop(data);
    readback.unmap();

    Ok(pixels)
}

fn save_pixels_to_png(
    pixels: Vec<[f32; 4]>,
    width: u32,
    height: u32,
    out_path: &Path,
) -> Result<()> {
    let mut img = image::ImageBuffer::new(width, height);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let idx = (y * width + x) as usize;
        let color = pixels[idx];
        *pixel = image::Rgba([
            (color[0].clamp(0.0, 1.0) * 255.0) as u8,
            (color[1].clamp(0.0, 1.0) * 255.0) as u8,
            (color[2].clamp(0.0, 1.0) * 255.0) as u8,
            (color[3].clamp(0.0, 1.0) * 255.0) as u8,
        ]);
    }
    img.save(out_path)?;
    Ok(())
}
