//! GPU buffer layouts.
use crate::gpu_types::{
    GpuBvhNode, GpuLight, GpuPrimitiveRef, GpuRenderParams, GpuSphere, GpuTriangle,
};
use crate::scene_upload::SceneGpuData;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct RenderParams {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub max_bounces: u32,
}

// Delete once the scene buffer is fully integrated
pub struct GradientBuffers {
    pub output: wgpu::Buffer,
    pub readback: wgpu::Buffer,
    pub params: wgpu::Buffer,
    pub output_size: u64,
    pub params_size: u64,
}

pub struct SceneGpuBuffers {
    pub output: wgpu::Buffer,
    pub readback: wgpu::Buffer,
    pub params: wgpu::Buffer,
    pub camera: wgpu::Buffer,
    pub spheres: wgpu::Buffer,
    pub triangles: wgpu::Buffer,
    pub materials: wgpu::Buffer,
    pub lights: wgpu::Buffer,
    pub bvh_nodes: wgpu::Buffer,
    pub bvh_primitives: wgpu::Buffer,
    pub output_size: u64,
    pub params_size: u64,
}

pub fn create_pixel_buffers(
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

pub fn create_scene_gpu_buffers(device: &wgpu::Device, scene: &SceneGpuData) -> SceneGpuBuffers {
    let pixel_count = scene.params.width as u64 * scene.params.height as u64;
    let output_size = pixel_count * std::mem::size_of::<[f32; 4]>() as u64;
    let params_size = std::mem::size_of::<GpuRenderParams>() as u64;

    let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Scene Output Buffer"),
        size: output_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Scene Readback Buffer"),
        size: output_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Scene Render Params Buffer"),
        contents: bytemuck::bytes_of(&scene.params),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Scene Camera Buffer"),
        contents: bytemuck::bytes_of(&scene.camera),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let dummy_sphere = [GpuSphere::zeroed()];
    let sphere_data = if scene.spheres.is_empty() {
        &dummy_sphere[..]
    } else {
        &scene.spheres
    };
    let spheres_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Scene Spheres Buffer"),
        contents: bytemuck::cast_slice(sphere_data),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });

    let dummy_triangle = [GpuTriangle::zeroed()];
    let triangle_data = if scene.triangles.is_empty() {
        &dummy_triangle[..]
    } else {
        &scene.triangles
    };
    let triangles_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Scene Triangles Buffer"),
        contents: bytemuck::cast_slice(triangle_data),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });

    let materials_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Scene Materials Buffer"),
        contents: bytemuck::cast_slice(&scene.materials),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });

    let dummy_light = [GpuLight::zeroed()];
    let light_data = if scene.lights.is_empty() {
        &dummy_light[..]
    } else {
        &scene.lights
    };
    let lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Scene Lights Buffer"),
        contents: bytemuck::cast_slice(light_data),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });

    let dummy_bvh_node = [GpuBvhNode::zeroed()];
    let bvh_node_data = if scene.bvh_nodes.is_empty() {
        &dummy_bvh_node[..]
    } else {
        &scene.bvh_nodes
    };
    let bvh_nodes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("BVH Nodes Buffer"),
        contents: bytemuck::cast_slice(bvh_node_data),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });

    let dummy_bvh_primitive = [GpuPrimitiveRef::zeroed()];
    let bvh_primitive_data = if scene.bvh_primitives.is_empty() {
        &dummy_bvh_primitive[..]
    } else {
        &scene.bvh_primitives
    };
    let bvh_primitives_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("BVH Primitives Buffer"),
        contents: bytemuck::cast_slice(bvh_primitive_data),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });

    SceneGpuBuffers {
        output: output_buffer,
        readback: readback_buffer,
        params: params_buffer,
        camera: camera_buffer,
        spheres: spheres_buffer,
        triangles: triangles_buffer,
        materials: materials_buffer,
        lights: lights_buffer,
        bvh_nodes: bvh_nodes_buffer,
        bvh_primitives: bvh_primitives_buffer,
        output_size,
        params_size,
    }
}
