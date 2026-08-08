//! Conversion from renderer-neutral scenes to shader-compatible GPU data.

use anyhow::{bail, Context, Result};
use glam::Vec3;
use std::path::Path;
use std::time::{Duration, Instant};
use toaster_bvh::{Aabb, FlatBvh, PrimitiveInfo, PrimitiveRef};
use toaster_scene::{Background, Material, Scene, Triangle};

use crate::gpu_types::{
    GpuBvhNode, GpuCamera, GpuLight, GpuMaterial, GpuPrimitiveRef, GpuRenderParams, GpuSphere,
    GpuTriangle, GpuTriangleAttributes,
};

/// Complete host-side representation of a scene ready for buffer creation.
pub struct SceneGpuData {
    /// Render dimensions, sampling controls, and resource counts.
    pub params: GpuRenderParams,
    /// Camera basis and image-plane geometry.
    pub camera: GpuCamera,
    /// Packed spheres.
    pub spheres: Vec<GpuSphere>,
    /// Packed triangle geometry.
    pub triangles: Vec<GpuTriangle>,
    /// Optional smooth-normal and texture-coordinate data parallel to triangles.
    pub triangle_attributes: Vec<GpuTriangleAttributes>,
    /// Packed material records.
    pub materials: Vec<GpuMaterial>,
    /// Emissive primitives used by direct-light sampling.
    pub lights: Vec<GpuLight>,
    /// Flattened hierarchy nodes used by CPU-equivalent GPU traversal.
    pub bvh_nodes: Vec<GpuBvhNode>,
    /// Primitive references stored in BVH leaf order.
    pub bvh_primitives: Vec<GpuPrimitiveRef>,
    /// Concatenated little-endian RGBA8 texture texels.
    pub texture_pixels: Vec<u32>,
    /// Linear HDR environment texels with importance weights in the alpha lane.
    pub environment_pixels: Vec<[f32; 4]>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// CPU wall-clock diagnostics for rebuilding renderer acceleration data.
pub(crate) struct ScenePackingTimings {
    /// Whether the direct-light list was rebuilt rather than reused.
    pub light_rebuilt: bool,
    /// Time spent rebuilding the direct-light sampling list.
    pub light_rebuild: Duration,
    /// Time spent rebuilding and flattening the mixed primitive BVH.
    pub bvh_rebuild: Duration,
}

/// Loads a JSON scene and converts it into [`SceneGpuData`].
pub fn load_scene_gpu(path: impl AsRef<Path>) -> Result<SceneGpuData> {
    let scene = toaster_scene::load_scene(path)?;
    scene_to_gpu(&scene)
}

/// Converts a complete scene, including static texture and environment pixels.
pub fn scene_to_gpu(scene: &Scene) -> Result<SceneGpuData> {
    scene_to_gpu_inner(scene, true, None).map(|(data, _)| data)
}

/// Converts per-frame mutable data while omitting invariant pixel payloads.
#[cfg(test)]
pub(crate) fn scene_to_gpu_frame(scene: &Scene) -> Result<SceneGpuData> {
    scene_to_gpu_frame_with_timings(scene, None).map(|(data, _)| data)
}

/// Converts mutable frame data and reports BVH/light-list rebuild costs.
pub(crate) fn scene_to_gpu_frame_with_timings(
    scene: &Scene,
    cached_lights: Option<(&[GpuLight], f32)>,
) -> Result<(SceneGpuData, ScenePackingTimings)> {
    scene_to_gpu_inner(scene, false, cached_lights)
}

/// Validates and packs a scene, optionally retaining static pixel arrays.
fn scene_to_gpu_inner(
    scene: &Scene,
    include_static_pixels: bool,
    cached_lights: Option<(&[GpuLight], f32)>,
) -> Result<(SceneGpuData, ScenePackingTimings)> {
    if scene.spheres.is_empty() && scene.triangles.is_empty() {
        bail!("GPU renderer requires at least one object");
    }
    if scene.materials.is_empty() {
        bail!("GPU renderer requires at least one material");
    }
    if scene.triangle_attributes.len() != scene.triangles.len() {
        bail!("triangle attribute count must match triangle count");
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

    let triangle_attributes = scene
        .triangle_attributes
        .iter()
        .map(|attributes| {
            let mut flags = 0_u32;
            let normals = attributes.normals.unwrap_or([Vec3::ZERO; 3]);
            if attributes.normals.is_some() {
                flags |= 1;
            }
            let tex_coords = attributes.tex_coords.unwrap_or([glam::Vec2::ZERO; 3]);
            if attributes.tex_coords.is_some() {
                flags |= 2;
            }
            GpuTriangleAttributes {
                n0: vec4(normals[0]),
                n1: vec4(normals[1]),
                n2: vec4(normals[2]),
                uv0: tex_coords[0].to_array(),
                uv1: tex_coords[1].to_array(),
                uv2: tex_coords[2].to_array(),
                flags,
                _pad0: 0,
            }
        })
        .collect::<Vec<_>>();

    let mut texture_pixels = Vec::new();
    let mut texture_metadata = Vec::with_capacity(scene.textures.len());
    let mut texture_pixel_count = 0_usize;
    for texture in &scene.textures {
        let offset = u32::try_from(texture_pixel_count).context("texture atlas is too large")?;
        texture_metadata.push((offset, texture.width, texture.height));
        let pixel_count = texture.rgba8.len() / 4;
        texture_pixel_count = texture_pixel_count
            .checked_add(pixel_count)
            .context("texture atlas is too large")?;
        if include_static_pixels {
            texture_pixels.extend(
                texture
                    .rgba8
                    .chunks_exact(4)
                    .map(|pixel| u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]])),
            );
        }
    }

    let materials = scene
        .materials
        .iter()
        .copied()
        .map(|material| material_to_gpu(material, &texture_metadata))
        .collect::<Result<Vec<_>>>()?;
    let (lights, total_light_area, light_rebuilt, light_rebuild) = match cached_lights {
        Some((lights, total_light_area)) => {
            (lights.to_vec(), total_light_area, false, Duration::ZERO)
        }
        None => {
            let light_start = Instant::now();
            let (lights, total_light_area) = lights_to_gpu(scene)?;
            (lights, total_light_area, true, light_start.elapsed())
        }
    };
    let light_count = u32::try_from(lights.len()).context("too many lights for GPU")?;
    let bvh_start = Instant::now();
    let (bvh_nodes, bvh_primitives) = build_gpu_bvh(scene)?;
    let bvh_rebuild = bvh_start.elapsed();
    let bvh_node_count = u32::try_from(bvh_nodes.len()).context("too many BVH nodes for GPU")?;
    let bvh_primitive_count =
        u32::try_from(bvh_primitives.len()).context("too many BVH primitives for GPU")?;

    let (
        environment_width,
        environment_height,
        environment_intensity,
        environment_rotation_degrees,
    ) = match (&scene.render.background, &scene.environment) {
        (Background::Environment, Some(environment)) => (
            environment.width,
            environment.height,
            environment.intensity,
            environment.rotation_degrees,
        ),
        (Background::Environment, None) => {
            bail!("environment background requires an environment map")
        }
        (_, _) => (0, 0, 0.0, 0.0),
    };
    let environment_pixels = if include_static_pixels {
        scene
            .environment
            .as_ref()
            .map(|environment| {
                environment
                    .pixels
                    .iter()
                    .zip(environment.importance_entries())
                    .map(|(pixel, importance)| [pixel.x, pixel.y, pixel.z, importance[0]])
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let data = SceneGpuData {
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
                Background::Environment => 2,
            },
            light_count,
            total_light_area,
            _pad0: 0,
            environment_width,
            environment_height,
            environment_intensity,
            environment_rotation_degrees,
            accumulated_samples: 0,
            _pad1: [0; 3],
            bvh_node_count,
            bvh_primitive_count,
            _pad2: [0; 2],
        },
        camera,
        spheres,
        triangles,
        triangle_attributes,
        materials,
        lights,
        bvh_nodes,
        bvh_primitives,
        texture_pixels,
        environment_pixels,
    };
    Ok((
        data,
        ScenePackingTimings {
            light_rebuilt,
            light_rebuild,
            bvh_rebuild,
        },
    ))
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

/// Builds the shader camera representation from look-at parameters.
///
/// Callers must supply a nondegenerate viewing direction and an `up` vector
/// that is not parallel to it.
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

/// Converts one material and resolves any texture index into atlas metadata.
fn material_to_gpu(
    material: Material,
    texture_metadata: &[(u32, u32, u32)],
) -> Result<GpuMaterial> {
    let (kind, albedo, params, texture) = match material {
        Material::Diffuse { albedo } => (0, albedo, [0.0; 4], None),
        Material::TexturedDiffuse {
            albedo,
            texture_index,
        } => (
            0,
            albedo,
            [0.0; 4],
            Some(
                *texture_metadata
                    .get(texture_index)
                    .context("material references an unknown texture")?,
            ),
        ),
        Material::Metal { albedo, roughness } => (1, albedo, [roughness, 0.0, 0.0, 0.0], None),
        Material::Dielectric { ior } => (2, Vec3::ONE, [0.0, ior, 0.0, 0.0], None),
        Material::Emissive { color, strength } => (3, color, [0.0, 0.0, strength, 0.0], None),
    };
    let (texture_offset, texture_width, texture_height) = texture.unwrap_or((0, 0, 0));

    Ok(GpuMaterial {
        kind,
        texture_offset,
        texture_width,
        texture_height,
        albedo: [albedo.x, albedo.y, albedo.z, 1.0],
        params,
    })
}

/// Expands a three-component vector to an aligned shader vector.
fn vec4(value: Vec3) -> [f32; 4] {
    [value.x, value.y, value.z, 0.0]
}

/// Collects emissive primitives and their cumulative area distribution.
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

/// Returns whether a material emits a positive amount of light.
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
    fn uploads_imported_gltf_triangles_without_a_special_render_path() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/008_gltf_tetrahedron.json");
        let scene = load_scene_gpu(path).unwrap();

        assert_eq!(scene.params.sphere_count, 1);
        assert_eq!(scene.params.triangle_count, 6);
        assert_eq!(scene.triangles[2].material_index, 1);
        assert_eq!(scene.triangles[2].v0, [0.0, 0.75, -2.0, 0.0]);
        assert_eq!(scene.params.light_count, 1);
    }

    #[test]
    fn uploads_textured_gltf_material_and_triangle_attributes() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/009_gltf_textured_quad.json");
        let scene = load_scene_gpu(path).unwrap();

        assert_eq!(scene.params.triangle_count, 4);
        assert_eq!(scene.triangle_attributes.len(), 4);
        assert_eq!(scene.triangle_attributes[0].flags, 3);
        assert_eq!(scene.triangle_attributes[0].n0, [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(scene.triangle_attributes[0].uv0, [0.0, 1.0]);
        assert_eq!(scene.materials[1].texture_offset, 0);
        assert_eq!(scene.materials[1].texture_width, 2);
        assert_eq!(scene.materials[1].texture_height, 2);
        assert_eq!(scene.texture_pixels.len(), 4);
        assert_eq!(scene.texture_pixels[0], 0xff00_00ff);
        assert_eq!(scene.params.light_count, 2);
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
            triangle_attributes: Vec::new(),
            textures: Vec::new(),
            environment: None,
            animation: Default::default(),
            physics: None,
            rigid_bodies: Vec::new(),
            triggers: Vec::new(),
        };
        let gpu_scene = scene_to_gpu(&scene).unwrap();

        assert!(gpu_scene.lights.is_empty());
        assert_eq!(gpu_scene.params.light_count, 0);
        assert_eq!(gpu_scene.params.total_light_area, 0.0);
    }

    #[test]
    fn uploads_float_environment_map_and_metadata() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/010_environment_map.json");
        let source = toaster_scene::load_scene(path).unwrap();
        let scene = scene_to_gpu(&source).unwrap();

        assert_eq!(scene.params.background_kind, 2);
        assert_eq!(scene.params.environment_width, 4);
        assert_eq!(scene.params.environment_height, 2);
        assert_eq!(scene.params.environment_intensity, 8.0);
        assert_eq!(scene.params.environment_rotation_degrees, 20.0);
        assert_eq!(scene.environment_pixels.len(), 8);
        assert!((scene.environment_pixels.last().unwrap()[3] - 1.0).abs() < 1e-6);
        assert!(scene
            .environment_pixels
            .iter()
            .any(|pixel| pixel[0] > 0.0 || pixel[1] > 0.0 || pixel[2] > 0.0));

        let frame = scene_to_gpu_frame(&source).unwrap();
        assert_eq!(frame.params.environment_width, 4);
        assert!(frame.environment_pixels.is_empty());
    }

    #[test]
    fn packs_neutrally_evaluated_physics_geometry_bvh_and_lights() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/011_physics_rigid_bodies.json");
        let mut source = toaster_scene::load_scene(path).unwrap();
        let sphere_body = source
            .rigid_bodies
            .iter()
            .find(|body| body.group.as_deref() == Some("ball_0"))
            .unwrap()
            .clone();
        let box_body = source
            .rigid_bodies
            .iter()
            .find(|body| body.group.as_deref() == Some("crate_0"))
            .unwrap()
            .clone();
        let sphere_index = match sphere_body.binding {
            toaster_scene::ObjectBinding::Sphere { index } => index,
            toaster_scene::ObjectBinding::Triangles { .. } => panic!("ball must bind a sphere"),
        };
        let box_start = match box_body.binding {
            toaster_scene::ObjectBinding::Triangles { start, .. } => start,
            toaster_scene::ObjectBinding::Sphere { .. } => panic!("crate must bind triangles"),
        };
        let sphere_material = source.spheres[sphere_index].material_index;
        source.materials[sphere_material] = Material::Emissive {
            color: Vec3::ONE,
            strength: 2.0,
        };
        let before = scene_to_gpu_frame(&source).unwrap();
        let mut evaluated = source.clone();
        toaster_scene::apply_rigid_transform(
            &source,
            &mut evaluated,
            &sphere_body.binding,
            toaster_scene::RigidTransform {
                translation: Vec3::new(20.0, 10.0, 0.0),
                rotation: glam::Quat::IDENTITY,
            },
        )
        .unwrap();
        toaster_scene::apply_rigid_transform(
            &source,
            &mut evaluated,
            &box_body.binding,
            toaster_scene::RigidTransform {
                translation: Vec3::new(-12.0, 4.0, -3.0),
                rotation: glam::Quat::from_rotation_y(0.7),
            },
        )
        .unwrap();
        let (after, packing) = scene_to_gpu_frame_with_timings(&evaluated, None).unwrap();

        assert_eq!(before.spheres.len(), after.spheres.len());
        assert_eq!(before.triangles.len(), after.triangles.len());
        assert_eq!(before.lights.len(), after.lights.len());
        assert_eq!(
            after.spheres[sphere_index].center_radius[..3],
            [20.0, 10.0, 0.0]
        );
        assert_ne!(
            before.triangles[box_start].v0,
            after.triangles[box_start].v0
        );
        let moved_light = after.lights.iter().find(|light| light.kind == 1).unwrap();
        assert_eq!(moved_light.center_radius[..3], [20.0, 10.0, 0.0]);
        assert_ne!(before.bvh_nodes[0].max, after.bvh_nodes[0].max);
        assert!(packing.light_rebuilt);
    }

    #[test]
    fn non_emissive_motion_reuses_the_light_list_but_rebuilds_the_bvh() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/011_physics_rigid_bodies.json");
        let source = toaster_scene::load_scene(path).unwrap();
        let before = scene_to_gpu(&source).unwrap();
        let box_body = source
            .rigid_bodies
            .iter()
            .find(|body| body.group.as_deref() == Some("crate_0"))
            .unwrap();
        let mut evaluated = source.clone();
        toaster_scene::apply_rigid_transform(
            &source,
            &mut evaluated,
            &box_body.binding,
            toaster_scene::RigidTransform {
                translation: Vec3::new(-12.0, 4.0, -3.0),
                rotation: glam::Quat::from_rotation_y(0.7),
            },
        )
        .unwrap();
        let (after, packing) = scene_to_gpu_frame_with_timings(
            &evaluated,
            Some((before.lights.as_slice(), before.params.total_light_area)),
        )
        .unwrap();

        assert!(!packing.light_rebuilt);
        assert_eq!(packing.light_rebuild, Duration::ZERO);
        assert_eq!(after.lights.len(), before.lights.len());
        assert_eq!(after.lights[0].v0, before.lights[0].v0);
        assert_ne!(after.bvh_nodes[0].min, before.bvh_nodes[0].min);
    }

    #[test]
    fn invisible_triggers_do_not_change_gpu_geometry_or_lights() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/013_physics_events_triggers.json");
        let source = toaster_scene::load_scene(path).unwrap();
        let packed = scene_to_gpu(&source).unwrap();

        assert_eq!(source.triggers.len(), 2);
        assert_eq!(packed.spheres.len(), source.spheres.len());
        assert_eq!(packed.triangles.len(), source.triangles.len());
        assert_eq!(
            packed.bvh_primitives.len(),
            source.spheres.len() + source.triangles.len()
        );
        assert_eq!(packed.lights.len(), 53);
    }
}
