//! GPU buffer layouts.
use crate::gpu_types::{
    GpuBvhNode, GpuLight, GpuPrimitiveRef, GpuRenderParams, GpuSphere, GpuTriangle,
    GpuTriangleAttributes,
};
use crate::scene_upload::SceneGpuData;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
/// Uniform parameters for the standalone gradient demonstration.
pub struct RenderParams {
    /// Output width.
    pub width: u32,
    /// Output height.
    pub height: u32,
    /// Demonstration sample field retained from the original scaffold.
    pub samples: u32,
    /// Demonstration bounce field retained from the original scaffold.
    pub max_bounces: u32,
}

/// Legacy resources used only by the standalone gradient demonstration.
pub struct GradientBuffers {
    /// Shader-writable float RGBA output.
    pub output: wgpu::Buffer,
    /// CPU-mappable copy destination.
    pub readback: wgpu::Buffer,
    /// Gradient parameter uniform.
    pub params: wgpu::Buffer,
    /// Output/readback size in bytes.
    pub output_size: u64,
    /// Uniform size in bytes.
    pub params_size: u64,
}

/// All persistent buffers bound by the active path-tracing pipeline.
pub struct SceneGpuBuffers {
    /// Shader-writable linear float RGBA accumulation buffer.
    pub output: wgpu::Buffer,
    /// CPU-mappable output copy.
    pub readback: wgpu::Buffer,
    /// Per-frame [`GpuRenderParams`] uniform.
    pub params: wgpu::Buffer,
    /// Per-frame camera uniform.
    pub camera: wgpu::Buffer,
    /// Sphere storage array or one dummy element.
    pub spheres: wgpu::Buffer,
    /// Triangle storage array or one dummy element.
    pub triangles: wgpu::Buffer,
    /// Per-triangle shading attributes or one dummy element.
    pub triangle_attributes: wgpu::Buffer,
    /// Material storage array.
    pub materials: wgpu::Buffer,
    /// Emissive primitive storage array or one dummy element.
    pub lights: wgpu::Buffer,
    /// Flattened BVH node storage array.
    pub bvh_nodes: wgpu::Buffer,
    /// BVH leaf primitive-reference storage array.
    pub bvh_primitives: wgpu::Buffer,
    /// Packed RGBA8 texture atlas or one dummy pixel.
    pub texture_pixels: wgpu::Buffer,
    /// Float RGB plus importance metadata environment texels or one dummy texel.
    pub environment_pixels: wgpu::Buffer,
    /// Output/readback size in bytes.
    pub output_size: u64,
    /// Render-parameter uniform size in bytes.
    pub params_size: u64,
}

/// Allocates and initializes the legacy gradient buffers.
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

/// Allocates the full path-tracing buffer set from converted scene data.
///
/// Empty optional storage arrays receive one zero/dummy element because wgpu
/// bindings cannot reference zero-sized buffers; shader count fields prevent use.
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

    let dummy_triangle_attributes = [GpuTriangleAttributes::zeroed()];
    let triangle_attribute_data = if scene.triangle_attributes.is_empty() {
        &dummy_triangle_attributes[..]
    } else {
        &scene.triangle_attributes
    };
    let triangle_attributes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Scene Triangle Attributes Buffer"),
        contents: bytemuck::cast_slice(triangle_attribute_data),
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

    // Animation can change whether centroid-degenerate ranges split. Allocate
    // the full binary-tree bound so later evaluated frames cannot outgrow the
    // initial node buffer even when their actual node count increases.
    let primitive_count = usize::try_from(scene.params.sphere_count)
        .expect("u32 sphere count fits usize")
        + usize::try_from(scene.params.triangle_count).expect("u32 triangle count fits usize");
    let bvh_node_capacity = primitive_count.saturating_mul(2).saturating_sub(1).max(1);
    debug_assert!(scene.bvh_nodes.len() <= bvh_node_capacity);
    let mut bvh_node_data = vec![GpuBvhNode::zeroed(); bvh_node_capacity];
    bvh_node_data[..scene.bvh_nodes.len()].copy_from_slice(&scene.bvh_nodes);
    let bvh_nodes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("BVH Nodes Buffer"),
        contents: bytemuck::cast_slice(&bvh_node_data),
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

    let dummy_texture_pixel = [0xffff_ffff_u32];
    let texture_data = if scene.texture_pixels.is_empty() {
        &dummy_texture_pixel[..]
    } else {
        &scene.texture_pixels
    };
    let texture_pixels_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Scene Texture Pixels Buffer"),
        contents: bytemuck::cast_slice(texture_data),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let dummy_environment_pixel = [[0.0_f32; 4]];
    let environment_data = if scene.environment_pixels.is_empty() {
        &dummy_environment_pixel[..]
    } else {
        &scene.environment_pixels
    };
    let environment_pixels_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Scene Environment Pixels Buffer"),
        contents: bytemuck::cast_slice(environment_data),
        usage: wgpu::BufferUsages::STORAGE,
    });

    SceneGpuBuffers {
        output: output_buffer,
        readback: readback_buffer,
        params: params_buffer,
        camera: camera_buffer,
        spheres: spheres_buffer,
        triangles: triangles_buffer,
        triangle_attributes: triangle_attributes_buffer,
        materials: materials_buffer,
        lights: lights_buffer,
        bvh_nodes: bvh_nodes_buffer,
        bvh_primitives: bvh_primitives_buffer,
        texture_pixels: texture_pixels_buffer,
        environment_pixels: environment_pixels_buffer,
        output_size,
        params_size,
    }
}
