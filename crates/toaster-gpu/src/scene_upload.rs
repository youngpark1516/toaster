use crate::gpu_types::{GpuCamera, GpuMaterial, GpuRenderParams, GpuSphere};
pub struct SceneGpuData {
    pub params: GpuRenderParams,
    pub camera: GpuCamera,
    pub spheres: Vec<GpuSphere>,
    pub materials: Vec<GpuMaterial>,
}

pub fn create_test_scene(width: u32, height: u32) -> SceneGpuData {
    SceneGpuData {
        params: GpuRenderParams {
            width,
            height,
            samples: 1,
            max_bounces: 1,
            sphere_count: 1,
            material_count: 1,
            frame_index: 0,
            _pad0: 0,
        },

        camera: GpuCamera {
            origin: [0.0, 0.0, 0.0, 0.0],
            lower_left_corner: [-2.0, -1.5, -1.0, 0.0],
            horizontal: [4.0, 0.0, 0.0, 0.0],
            vertical: [0.0, 3.0, 0.0, 0.0],
        },

        spheres: vec![GpuSphere {
            center_radius: [0.0, 0.0, -1.0, 0.5],
            material_index: 0,
            _pad0: [0; 3],
        }],

        materials: vec![GpuMaterial {
            kind: 0,
            _pad0: [0; 3],
            albedo: [0.8, 0.3, 0.3, 1.0],
            params: [0.0, 0.0, 0.0, 0.0],
        }],
    }
}
