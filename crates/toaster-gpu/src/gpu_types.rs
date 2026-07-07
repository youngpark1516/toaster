use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuRenderParams {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub max_bounces: u32,

    pub sphere_count: u32,
    pub material_count: u32,
    pub frame_index: u32,
    pub background_kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuCamera {
    pub origin: [f32; 4],
    pub lower_left_corner: [f32; 4],
    pub horizontal: [f32; 4],
    pub vertical: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuSphere {
    // xyz = center, w = radius
    pub center_radius: [f32; 4],

    pub material_index: u32,
    pub _pad0: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuMaterial {
    // 0 = diffuse, 1 = metal, 2 = dielectric, 3 = emissive
    pub kind: u32,
    pub _pad0: [u32; 3],

    pub albedo: [f32; 4],

    // x = roughness, y = ior, z = emission_strength, w = unused
    pub params: [f32; 4],
}
