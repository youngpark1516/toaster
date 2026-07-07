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
            samples: 32,
            max_bounces: 16,
            sphere_count: 4,
            material_count: 4,
            frame_index: 0,
            _pad0: 0,
        },

        camera: GpuCamera {
            origin: [0.0, 1.4, 6.0, 0.0],
            lower_left_corner: [-0.647_058, 0.891_716, 5.065_06, 0.0],
            horizontal: [1.294_116, 0.0, 0.0, 0.0],
            vertical: [0.0, 0.719_887, -0.107_983, 0.0],
        },

        spheres: vec![
            // Ground
            GpuSphere {
                center_radius: [0.0, -1000.0, 0.0, 1000.0],
                material_index: 0,
                _pad0: [0; 3],
            },
            // Left: matte blue
            GpuSphere {
                center_radius: [-1.2, 0.5, 0.0, 0.5],
                material_index: 1,
                _pad0: [0; 3],
            },
            // Middle: glass
            GpuSphere {
                center_radius: [0.0, 0.5, 0.0, 0.5],
                material_index: 2,
                _pad0: [0; 3],
            },
            // Right: metal
            GpuSphere {
                center_radius: [1.2, 0.5, 0.0, 0.5],
                material_index: 3,
                _pad0: [0; 3],
            },
        ],

        materials: vec![
            // Ground
            GpuMaterial {
                kind: 0,
                _pad0: [0; 3],
                albedo: [0.55, 0.55, 0.55, 1.0],
                params: [0.0, 0.0, 0.0, 0.0],
            },
            // Matte blue
            GpuMaterial {
                kind: 0,
                _pad0: [0; 3],
                albedo: [0.15, 0.35, 0.8, 1.0],
                params: [0.0, 0.0, 0.0, 0.0],
            },
            // Glass
            GpuMaterial {
                kind: 2,
                _pad0: [0; 3],
                albedo: [1.0, 1.0, 1.0, 1.0],
                params: [0.0, 1.5, 0.0, 0.0], // y = IOR
            },
            // Metal
            GpuMaterial {
                kind: 1,
                _pad0: [0; 3],
                albedo: [0.85, 0.75, 0.55, 1.0],
                params: [0.08, 0.0, 0.0, 0.0], // x = roughness
            },
        ],
    }
}
