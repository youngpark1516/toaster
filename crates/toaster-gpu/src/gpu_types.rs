use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuRenderParams {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub max_bounces: u32,

    pub sphere_count: u32,
    pub triangle_count: u32,
    pub material_count: u32,
    pub frame_index: u32,

    pub background_kind: u32,
    pub light_count: u32,
    pub total_light_area: f32,
    pub _pad0: u32,

    pub environment_width: u32,
    pub environment_height: u32,
    pub environment_intensity: f32,
    pub environment_rotation_degrees: f32,

    pub accumulated_samples: u32,
    pub _pad1: [u32; 3],
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
pub struct GpuTriangle {
    pub v0: [f32; 4],
    pub v1: [f32; 4],
    pub v2: [f32; 4],

    pub material_index: u32,
    pub _pad0: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuTriangleAttributes {
    pub n0: [f32; 4],
    pub n1: [f32; 4],
    pub n2: [f32; 4],

    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    pub uv2: [f32; 2],
    pub flags: u32,
    pub _pad0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuMaterial {
    // 0 = diffuse, 1 = metal, 2 = dielectric, 3 = emissive
    pub kind: u32,
    pub texture_offset: u32,
    pub texture_width: u32,
    pub texture_height: u32,

    pub albedo: [f32; 4],

    // x = roughness, y = ior, z = emission_strength, w = unused
    pub params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuLight {
    // 0 = triangle, 1 = sphere
    pub kind: u32,
    pub material_index: u32,
    pub _pad0: [u32; 2],

    pub v0: [f32; 4],
    pub v1: [f32; 4],
    pub v2: [f32; 4],
    pub center_radius: [f32; 4],

    // x = area, y = cumulative_area, z/w = unused
    pub area_cumulative: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_types_match_wgsl_layout_sizes() {
        assert_eq!(std::mem::size_of::<GpuRenderParams>(), 80);
        assert_eq!(std::mem::size_of::<GpuCamera>(), 64);
        assert_eq!(std::mem::size_of::<GpuSphere>(), 32);
        assert_eq!(std::mem::size_of::<GpuTriangle>(), 64);
        assert_eq!(std::mem::size_of::<GpuTriangleAttributes>(), 80);
        assert_eq!(std::mem::size_of::<GpuMaterial>(), 48);
        assert_eq!(std::mem::size_of::<GpuLight>(), 96);
    }
}
