use anyhow::{bail, Context, Result};
use glam::Vec3;
use std::path::Path;
use toaster_scene::{Background, Material, Scene};

use crate::gpu_types::{GpuCamera, GpuMaterial, GpuRenderParams, GpuSphere};

pub struct SceneGpuData {
    pub params: GpuRenderParams,
    pub camera: GpuCamera,
    pub spheres: Vec<GpuSphere>,
    pub materials: Vec<GpuMaterial>,
}

pub fn load_scene_gpu(path: impl AsRef<Path>) -> Result<SceneGpuData> {
    let scene = toaster_scene::load_scene(path)?;
    scene_to_gpu(&scene)
}

pub fn scene_to_gpu(scene: &Scene) -> Result<SceneGpuData> {
    if !scene.triangles.is_empty() {
        bail!("GPU renderer currently supports spheres only; scene contains triangles");
    }
    if scene.spheres.is_empty() {
        bail!("GPU renderer requires at least one sphere");
    }
    if scene.materials.is_empty() {
        bail!("GPU renderer requires at least one material");
    }

    let sphere_count = u32::try_from(scene.spheres.len()).context("too many spheres for GPU")?;
    let material_count =
        u32::try_from(scene.materials.len()).context("too many materials for GPU")?;

    let aspect_ratio = scene.render.width as f32 / scene.render.height as f32;
    let camera = make_camera(
        scene.camera.position,
        scene.camera.look_at,
        scene.camera.up,
        scene.camera.fov_degrees,
        aspect_ratio,
    );

    let spheres = scene
        .spheres
        .iter()
        .map(|sphere| {
            Ok(GpuSphere {
                center_radius: [
                    sphere.center.x,
                    sphere.center.y,
                    sphere.center.z,
                    sphere.radius,
                ],
                material_index: u32::try_from(sphere.material_index)
                    .context("material index does not fit on GPU")?,
                _pad0: [0; 3],
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let materials = scene
        .materials
        .iter()
        .copied()
        .map(material_to_gpu)
        .collect();

    Ok(SceneGpuData {
        params: GpuRenderParams {
            width: scene.render.width,
            height: scene.render.height,
            samples: scene.render.samples,
            max_bounces: scene.render.max_bounces,
            sphere_count,
            material_count,
            frame_index: 0,
            background_kind: match scene.render.background {
                Background::Sky => 0,
                Background::Black => 1,
            },
        },
        camera,
        spheres,
        materials,
    })
}

fn make_camera(position: Vec3, look_at: Vec3, up: Vec3, fov: f32, aspect: f32) -> GpuCamera {
    let backward = (position - look_at).normalize();
    let right = up.cross(backward).normalize();
    let true_up = backward.cross(right);
    let viewport_height = 2.0 * (0.5 * fov.to_radians()).tan();
    let viewport_width = aspect * viewport_height;
    let horizontal = viewport_width * right;
    let vertical = viewport_height * true_up;
    let lower_left = position - horizontal * 0.5 - vertical * 0.5 - backward;

    GpuCamera {
        origin: vec4(position),
        lower_left_corner: vec4(lower_left),
        horizontal: vec4(horizontal),
        vertical: vec4(vertical),
    }
}

fn material_to_gpu(material: Material) -> GpuMaterial {
    let (kind, albedo, params) = match material {
        Material::Diffuse { albedo } => (0, albedo, [0.0; 4]),
        Material::Metal { albedo, roughness } => (1, albedo, [roughness, 0.0, 0.0, 0.0]),
        Material::Dielectric { ior } => (2, Vec3::ONE, [0.0, ior, 0.0, 0.0]),
        Material::Emissive { color, strength } => (3, color, [0.0, 0.0, strength, 0.0]),
    };

    GpuMaterial {
        kind,
        _pad0: [0; 3],
        albedo: [albedo.x, albedo.y, albedo.z, 1.0],
        params,
    }
}

fn vec4(value: Vec3) -> [f32; 4] {
    [value.x, value.y, value.z, 0.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_materials_scene_into_gpu_layout() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/002_materials.json");
        let scene = load_scene_gpu(path).unwrap();

        assert_eq!((scene.params.width, scene.params.height), (800, 450));
        assert_eq!((scene.params.samples, scene.params.max_bounces), (32, 16));
        assert_eq!(
            (scene.params.sphere_count, scene.params.material_count),
            (5, 5)
        );
        assert_eq!(scene.camera.origin, [0.0, 1.4, 6.0, 0.0]);
        assert_eq!(scene.spheres[3].center_radius, [1.2, 0.5, 0.0, 0.5]);
        assert_eq!(scene.materials[2].kind, 2);
        assert_eq!(scene.materials[2].params[1], 1.5);
        assert_eq!(scene.materials[3].kind, 1);
        assert_eq!(scene.materials[3].params[0], 0.08);
        assert_eq!(scene.materials[4].kind, 3);
        assert_eq!(scene.materials[4].params[2], 6.0);
    }
}
