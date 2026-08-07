use anyhow::{bail, Context, Result};
use glam::Vec3;
use std::path::Path;
use toaster_bvh::{Aabb, FlatBvh, PrimitiveInfo, PrimitiveRef};
use toaster_scene::{Background, Material, Scene, Triangle};

use crate::gpu_types::{
    GpuBvhNode, GpuCamera, GpuLight, GpuMaterial, GpuPrimitiveRef, GpuRenderParams, GpuSphere,
    GpuTriangle,
};

pub struct SceneGpuData {
    pub params: GpuRenderParams,
    pub camera: GpuCamera,
    pub spheres: Vec<GpuSphere>,
    pub triangles: Vec<GpuTriangle>,
    pub materials: Vec<GpuMaterial>,
    pub lights: Vec<GpuLight>,
    pub bvh_nodes: Vec<GpuBvhNode>,
    pub bvh_primitives: Vec<GpuPrimitiveRef>,
}

pub fn load_scene_gpu(path: impl AsRef<Path>) -> Result<SceneGpuData> {
    let scene = toaster_scene::load_scene(path)?;
    scene_to_gpu(&scene)
}

pub fn scene_to_gpu(scene: &Scene) -> Result<SceneGpuData> {
    if scene.spheres.is_empty() && scene.triangles.is_empty() {
        bail!("GPU renderer requires at least one object");
    }
    if scene.materials.is_empty() {
        bail!("GPU renderer requires at least one material");
    }

    let sphere_count = u32::try_from(scene.spheres.len()).context("too many spheres for GPU")?;
    let triangle_count =
        u32::try_from(scene.triangles.len()).context("too many triangles for GPU")?;
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

    let triangles = scene
        .triangles
        .iter()
        .map(|triangle| {
            Ok(GpuTriangle {
                v0: vec4(triangle.vertices[0]),
                v1: vec4(triangle.vertices[1]),
                v2: vec4(triangle.vertices[2]),
                material_index: u32::try_from(triangle.material_index)
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
    let (lights, total_light_area) = lights_to_gpu(scene)?;
    let light_count = u32::try_from(lights.len()).context("too many lights for GPU")?;

    let (bvh_nodes, bvh_primitives) = build_gpu_bvh(scene)?;
    let bvh_node_count = u32::try_from(bvh_nodes.len()).context("too many BVH nodes for GPU")?;
    let bvh_primitive_count =
        u32::try_from(bvh_primitives.len()).context("too many BVH primitives for GPU")?;

    Ok(SceneGpuData {
        params: GpuRenderParams {
            width: scene.render.width,
            height: scene.render.height,
            samples: scene.render.samples,
            max_bounces: scene.render.max_bounces,
            sphere_count,
            triangle_count,
            material_count,
            frame_index: 0,
            background_kind: match scene.render.background {
                Background::Sky => 0,
                Background::Black => 1,
            },
            light_count,
            total_light_area,
            _pad0: 0,
            bvh_node_count,
            bvh_primitive_count,
        },
        camera,
        spheres,
        triangles,
        materials,
        lights,
        bvh_nodes,
        bvh_primitives,
    })
}

fn build_gpu_bvh(scene: &Scene) -> Result<(Vec<GpuBvhNode>, Vec<GpuPrimitiveRef>)> {
    let mut primitive_info = Vec::with_capacity(scene.spheres.len() + scene.triangles.len());

    primitive_info.extend(scene.spheres.iter().enumerate().map(|(index, sphere)| {
        let radius = Vec3::splat(sphere.radius);
        let bounds = Aabb::new(sphere.center - radius, sphere.center + radius);
        PrimitiveInfo {
            bounds,
            centroid: bounds.centroid(),
            primitive: PrimitiveRef::Sphere(index as u32),
        }
    }));

    primitive_info.extend(scene.triangles.iter().enumerate().map(|(index, triangle)| {
        let bounds = triangle_bounds(triangle);
        PrimitiveInfo {
            bounds,
            centroid: bounds.centroid(),
            primitive: PrimitiveRef::Triangle(index as u32),
        }
    }));

    let flat = FlatBvh::build(&mut primitive_info);
    let nodes = flat
        .nodes
        .into_iter()
        .map(|node| GpuBvhNode {
            min: [node.bounds.min.x, node.bounds.min.y, node.bounds.min.z, 0.0],
            max: [node.bounds.max.x, node.bounds.max.y, node.bounds.max.z, 0.0],
            first_or_right: node.first_primitive_or_right_child,
            primitive_count: node.primitive_count,
            _pad: [0; 2],
        })
        .collect();
    let primitives = flat
        .primitives
        .into_iter()
        .map(|primitive| match primitive {
            PrimitiveRef::Sphere(index) => GpuPrimitiveRef { kind: 0, index },
            PrimitiveRef::Triangle(index) => GpuPrimitiveRef { kind: 1, index },
        })
        .collect();

    Ok((nodes, primitives))
}

fn triangle_bounds(triangle: &Triangle) -> Aabb {
    const PADDING: f32 = 1e-5;
    let [first, second, third] = triangle.vertices;
    Aabb::new(first.min(second).min(third), first.max(second).max(third)).expand(PADDING)
}

pub fn make_camera(position: Vec3, look_at: Vec3, up: Vec3, fov: f32, aspect: f32) -> GpuCamera {
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

fn lights_to_gpu(scene: &Scene) -> Result<(Vec<GpuLight>, f32)> {
    let mut cumulative_area = 0.0;
    let mut lights = Vec::new();

    for triangle in &scene.triangles {
        if !is_emissive(scene.materials[triangle.material_index]) {
            continue;
        }

        let edge1 = triangle.vertices[1] - triangle.vertices[0];
        let edge2 = triangle.vertices[2] - triangle.vertices[0];
        let area = 0.5 * edge1.cross(edge2).length();
        if area <= 0.0 {
            continue;
        }

        cumulative_area += area;
        lights.push(GpuLight {
            kind: 0,
            material_index: u32::try_from(triangle.material_index)
                .context("material index does not fit on GPU")?,
            _pad0: [0; 2],
            v0: vec4(triangle.vertices[0]),
            v1: vec4(triangle.vertices[1]),
            v2: vec4(triangle.vertices[2]),
            center_radius: [0.0; 4],
            area_cumulative: [area, cumulative_area, 0.0, 0.0],
        });
    }

    for sphere in &scene.spheres {
        if !is_emissive(scene.materials[sphere.material_index]) {
            continue;
        }

        let area = 4.0 * std::f32::consts::PI * sphere.radius * sphere.radius;
        if area <= 0.0 {
            continue;
        }

        cumulative_area += area;
        lights.push(GpuLight {
            kind: 1,
            material_index: u32::try_from(sphere.material_index)
                .context("material index does not fit on GPU")?,
            _pad0: [0; 2],
            v0: [0.0; 4],
            v1: [0.0; 4],
            v2: [0.0; 4],
            center_radius: [
                sphere.center.x,
                sphere.center.y,
                sphere.center.z,
                sphere.radius,
            ],
            area_cumulative: [area, cumulative_area, 0.0, 0.0],
        });
    }

    Ok((lights, cumulative_area))
}

fn is_emissive(material: Material) -> bool {
    matches!(material, Material::Emissive { strength, .. } if strength > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_materials_scene_into_gpu_layout() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/002_materials.json");
        let source_scene = toaster_scene::load_scene(&path).unwrap();
        let scene = load_scene_gpu(path).unwrap();

        assert_eq!(
            (scene.params.width, scene.params.height),
            (source_scene.render.width, source_scene.render.height)
        );
        assert_eq!(
            (scene.params.samples, scene.params.max_bounces),
            (source_scene.render.samples, source_scene.render.max_bounces)
        );
        assert_eq!(
            (
                scene.params.sphere_count,
                scene.params.triangle_count,
                scene.params.material_count
            ),
            (
                source_scene.spheres.len() as u32,
                source_scene.triangles.len() as u32,
                source_scene.materials.len() as u32
            )
        );
        assert_eq!(scene.camera.origin, [0.0, 1.4, 6.0, 0.0]);
        assert_eq!(scene.spheres[3].center_radius, [1.2, 0.5, 0.0, 0.5]);
        assert_eq!(scene.materials[2].kind, 2);
        assert_eq!(scene.materials[2].params[1], 1.5);
        assert_eq!(scene.materials[3].kind, 1);
        assert_eq!(scene.materials[3].params[0], 0.08);
        assert_eq!(scene.materials[4].kind, 3);
        assert_eq!(scene.materials[4].params[2], 6.0);
        assert_eq!(scene.triangles.len(), source_scene.triangles.len());
        assert_eq!(scene.lights.len(), 1);
        assert_eq!(scene.params.light_count, 1);
        assert!(scene.params.total_light_area > 0.0);
        assert_eq!(scene.lights[0].kind, 1);
        assert_eq!(scene.lights[0].material_index, 4);
    }

    #[test]
    fn loads_cornell_box_triangles_into_gpu_layout() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/003_cornell_box.json");
        let scene = load_scene_gpu(path).unwrap();

        assert_eq!(scene.triangles.len(), 12);
        assert_eq!(scene.triangles[0].v0, [-2.0, 0.0, 0.0, 0.0]);
        assert_eq!(scene.triangles[0].v1, [2.0, 0.0, -4.0, 0.0]);
        assert_eq!(scene.triangles[0].material_index, 0);
        assert_eq!(scene.triangles[6].material_index, 1);
        assert_eq!(scene.triangles[8].material_index, 2);
        assert_eq!(scene.triangles[10].material_index, 3);
        assert_eq!(scene.lights.len(), 2);
        assert_eq!(scene.params.light_count, 2);
        assert!(scene.params.total_light_area > 0.0);
        assert!(scene.lights.iter().all(|light| light.kind == 0));
        assert!(!scene.bvh_nodes.is_empty());
        assert_eq!(scene.params.bvh_node_count, scene.bvh_nodes.len() as u32);
        assert_eq!(
            scene.params.bvh_primitive_count,
            scene.bvh_primitives.len() as u32
        );
    }

    #[test]
    fn loads_mesh_room_into_gpu_layout() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/004_mesh.json");
        let source = toaster_scene::load_scene(&path).unwrap();
        let evaluated = source.evaluate_at(0.5).unwrap();
        let scene = load_scene_gpu(path).unwrap();

        assert_eq!(scene.spheres.len(), 0);
        assert_eq!(scene.triangles.len(), 24);
        assert_eq!(
            source
                .triangles
                .iter()
                .filter(|triangle| triangle.group.as_deref() == Some("cube"))
                .count(),
            12
        );
        assert_eq!(source.animation.tracks.len(), 2);
        assert_eq!(
            evaluated.triangles[0].vertices,
            source.triangles[0].vertices
        );
        assert_ne!(
            evaluated.triangles[12].vertices,
            source.triangles[12].vertices
        );
        assert_ne!(evaluated.camera.position, source.camera.position);
        assert_eq!(evaluated.camera.look_at, source.camera.look_at);
        assert_eq!(
            (
                scene.params.sphere_count,
                scene.params.triangle_count,
                scene.params.material_count
            ),
            (0, 24, 6)
        );
        assert_eq!(scene.materials[3].kind, 0);
        assert_eq!(scene.materials[4].kind, 1);
        assert_eq!(scene.materials[5].kind, 3);
        assert_eq!(scene.lights.len(), 2);
        assert_eq!(scene.params.light_count, 2);
        assert!(scene.params.total_light_area > 0.0);
        assert!(scene.lights.iter().all(|light| light.kind == 0));
        assert!(!scene.bvh_nodes.is_empty());
        assert_eq!(scene.params.bvh_node_count, scene.bvh_nodes.len() as u32);
        assert_eq!(scene.params.bvh_primitive_count, 24);
    }

    #[test]
    fn uploads_no_lights_scene_with_zero_light_count() {
        let scene = Scene {
            camera: toaster_scene::CameraSettings {
                position: Vec3::new(0.0, 0.0, 1.0),
                look_at: Vec3::ZERO,
                up: Vec3::Y,
                fov_degrees: 45.0,
            },
            render: toaster_scene::RenderSettings {
                width: 4,
                height: 4,
                samples: 1,
                max_bounces: 1,
                background: Background::Black,
            },
            materials: vec![Material::Diffuse { albedo: Vec3::ONE }],
            spheres: vec![toaster_scene::Sphere {
                center: Vec3::ZERO,
                radius: 0.5,
                material_index: 0,
                group: None,
            }],
            triangles: Vec::new(),
            animation: Default::default(),
        };
        let gpu_scene = scene_to_gpu(&scene).unwrap();

        assert!(gpu_scene.lights.is_empty());
        assert_eq!(gpu_scene.params.light_count, 0);
        assert_eq!(gpu_scene.params.total_light_area, 0.0);
    }
}
