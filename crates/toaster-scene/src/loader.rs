//! JSON scene deserialization, relative asset resolution, and validation.

use crate::{
    animation::{validate_animation, Animation},
    environment::EnvironmentMap,
    material::Material,
    object::{Sphere, Triangle, TriangleAttributes},
    physics::{
        ColliderShape, ObjectBinding, PhysicsSettings, PhysicsType, RigidBodyDeclaration,
        RigidBodyKind,
    },
    scene::{Background, CameraSettings, RenderSettings, Scene},
    texture::Texture,
};
use anyhow::{bail, Context, Result};
use glam::Vec3;
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
/// Raw top-level JSON representation before validation and asset expansion.
struct SceneFile {
    /// Camera input.
    camera: CameraFile,
    /// Render input.
    render: RenderFile,
    /// Named material declarations.
    materials: Vec<MaterialFile>,
    /// Primitive and mesh declarations.
    objects: Vec<ObjectFile>,
    /// Optional animation tracks.
    #[serde(default)]
    animation: Animation,
    /// Optional renderer-neutral physics-world declaration.
    physics: Option<PhysicsFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Raw top-level physics controls.
struct PhysicsFile {
    /// Whether physics participates in evaluation and ownership checks.
    #[serde(default = "default_true")]
    enabled: bool,
    /// Simulation domain; only rigid bodies are accepted in the MVP.
    #[serde(rename = "type")]
    physics_type: PhysicsTypeFile,
    /// World-space acceleration.
    #[serde(default = "default_gravity")]
    gravity: Vec3,
    /// Nominal fixed timestep in seconds.
    #[serde(default = "default_physics_timestep")]
    timestep: f32,
    /// Backend steps within one nominal tick.
    #[serde(default = "default_physics_substeps")]
    substeps: u32,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Physics simulation domains accepted by this scene version.
enum PhysicsTypeFile {
    /// Discrete rigid bodies.
    RigidBody,
}

#[derive(Deserialize)]
/// Raw camera fields accepted from JSON.
struct CameraFile {
    /// Camera position.
    position: Vec3,
    /// Look-at point.
    look_at: Vec3,
    /// Optional up vector, defaulting to positive Y.
    #[serde(default = "default_up")]
    up: Vec3,
    /// Vertical field of view, including its legacy alias.
    #[serde(alias = "vertical_fov_degrees")]
    fov_degrees: f32,
}

#[derive(Deserialize)]
/// Raw render settings accepted from JSON.
struct RenderFile {
    /// Output width.
    width: u32,
    /// Output height.
    height: u32,
    /// Samples per pixel, including its legacy alias.
    #[serde(alias = "samples_per_pixel")]
    samples: u32,
    /// Maximum path depth.
    max_bounces: u32,
    /// Sky, black, or detailed environment background.
    #[serde(default)]
    background: BackgroundFile,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
/// A compact named background or a detailed background object.
enum BackgroundFile {
    /// `"sky"` or `"black"`.
    Kind(BackgroundKindFile),
    /// Structured environment description.
    Detailed(DetailedBackgroundFile),
}

impl Default for BackgroundFile {
    /// Uses the procedural sky when a scene omits its background.
    fn default() -> Self {
        Self::Kind(BackgroundKindFile::Sky)
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Compact non-image background choices.
enum BackgroundKindFile {
    /// Procedural sky gradient.
    #[default]
    Sky,
    /// Zero-radiance background.
    Black,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Structured background variants.
enum DetailedBackgroundFile {
    /// Equirectangular image lighting and background.
    Environment {
        /// Absolute path or path relative to the scene file.
        path: PathBuf,
        /// Nonnegative radiance multiplier.
        #[serde(default = "default_environment_intensity")]
        intensity: f32,
        /// Optional yaw rotation in degrees.
        #[serde(default)]
        rotation_degrees: f32,
    },
}

#[derive(Deserialize)]
/// Named material declaration used by object references.
struct MaterialFile {
    /// Unique nonempty lookup name.
    name: String,
    /// Tagged material payload.
    #[serde(flatten)]
    material: MaterialData,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Material variants accepted in scene JSON.
enum MaterialData {
    /// Constant Lambertian base color.
    Diffuse {
        /// RGB value in `[0, 1]`.
        albedo: Vec3,
    },
    /// Fuzzy specular reflector.
    Metal {
        /// RGB reflection tint in `[0, 1]`.
        albedo: Vec3,
        /// Finite value clamped into `[0, 1]`.
        roughness: f32,
    },
    /// Ideal glass-like material.
    Dielectric {
        /// Positive index of refraction.
        ior: f32,
    },
    /// Surface light source.
    Emissive {
        /// RGB emission color in `[0, 1]`.
        color: Vec3,
        /// Nonnegative emission multiplier.
        strength: f32,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Object variants accepted in scene JSON.
enum ObjectFile {
    /// Analytic sphere.
    Sphere {
        /// World-space center.
        center: Vec3,
        /// Positive radius.
        radius: f32,
        /// Name of a declared Toaster material.
        material: String,
        /// Optional animation group.
        group: Option<String>,
        /// Optional renderer-neutral rigid-body declaration.
        physics: Option<RigidBodyFile>,
    },
    /// Generated axis-aligned box expanded into triangles during loading.
    Box {
        /// World-space box center.
        center: Vec3,
        /// Positive full extents on all axes.
        size: Vec3,
        /// Name of a declared Toaster material.
        material: String,
        /// Optional animation group.
        group: Option<String>,
        /// Optional renderer-neutral rigid-body declaration.
        physics: Option<RigidBodyFile>,
    },
    /// Explicit triangle.
    Triangle {
        /// Three finite non-collinear vertices.
        vertices: [Vec3; 3],
        /// Name of a declared Toaster material.
        material: String,
        /// Optional animation group.
        group: Option<String>,
        /// Rejected in the rigid-body MVP.
        physics: Option<RigidBodyFile>,
    },
    /// Imported glTF/GLB triangle mesh.
    Mesh {
        /// Absolute path or path relative to the scene file.
        path: PathBuf,
        /// Optional Toaster material overriding imported base colors.
        material: Option<String>,
        /// Optional animation group assigned to all imported triangles.
        group: Option<String>,
        /// Rejected in the rigid-body MVP.
        physics: Option<RigidBodyFile>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Raw per-object rigid-body properties.
struct RigidBodyFile {
    /// Static or dynamic ownership.
    body: RigidBodyKindFile,
    /// Optional explicit shape assertion.
    collider: Option<ColliderKindFile>,
    /// Optional dynamic mass.
    mass: Option<f32>,
    /// Coulomb friction coefficient.
    #[serde(default = "default_friction")]
    friction: f32,
    /// Contact restitution coefficient.
    #[serde(default)]
    restitution: f32,
    /// Optional initial linear velocity.
    initial_velocity: Option<Vec3>,
    /// Optional initial angular velocity.
    initial_angular_velocity: Option<Vec3>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Serialized rigid-body kinds.
enum RigidBodyKindFile {
    /// Immovable body.
    Static,
    /// Simulated body.
    Dynamic,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Serialized collider assertions.
enum ColliderKindFile {
    /// Sphere collider.
    Sphere,
    /// Box collider.
    Cuboid,
}

/// Supplies positive Y as the default camera up vector.
fn default_up() -> Vec3 {
    Vec3::Y
}

/// Supplies unit intensity as the default environment multiplier.
fn default_environment_intensity() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_gravity() -> Vec3 {
    Vec3::new(0.0, -9.81, 0.0)
}

fn default_physics_timestep() -> f32 {
    1.0 / 60.0
}

fn default_physics_substeps() -> u32 {
    1
}

fn default_friction() -> f32 {
    0.5
}

/// Loads, parses, validates, and expands a JSON scene and its relative assets.
pub fn load_scene(path: impl AsRef<Path>) -> Result<Scene> {
    let path = path.as_ref();
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read scene {}", path.display()))?;
    let file: SceneFile = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse scene {}", path.display()))?;
    build_scene(file, path.parent().unwrap_or_else(|| Path::new(".")))
}

/// Converts deserialized input into renderer-neutral runtime structures.
///
/// Imported mesh geometry is expanded into the scene's triangle arrays, while
/// imported textures and materials are rebased into scene-global indices.
fn build_scene(file: SceneFile, asset_root: &Path) -> Result<Scene> {
    if file.render.width == 0 || file.render.height == 0 {
        bail!("render width and height must be greater than zero");
    }
    if file.render.samples == 0 {
        bail!("render samples must be greater than zero");
    }
    if !(0.0..180.0).contains(&file.camera.fov_degrees) {
        bail!("camera fov_degrees must be between 0 and 180");
    }
    if !file.camera.position.is_finite()
        || !file.camera.look_at.is_finite()
        || !file.camera.up.is_finite()
        || file
            .camera
            .position
            .abs_diff_eq(file.camera.look_at, f32::EPSILON)
    {
        bail!("camera vectors are invalid");
    }
    let view = file.camera.position - file.camera.look_at;
    if file.camera.up.cross(view).length_squared() <= f32::EPSILON {
        bail!("camera up vector must not be parallel to its view direction");
    }

    let physics = file.physics.map(validate_physics_settings).transpose()?;

    let mut material_names = HashMap::new();
    let mut materials = Vec::with_capacity(file.materials.len());
    for material in file.materials {
        if material_names.contains_key(&material.name) {
            bail!("duplicate material name '{}'", material.name);
        }

        let runtime_material = match material.material {
            MaterialData::Diffuse { albedo } => {
                validate_albedo(&material.name, albedo)?;
                Material::Diffuse { albedo }
            }
            MaterialData::Metal { albedo, roughness } => {
                validate_albedo(&material.name, albedo)?;
                if !roughness.is_finite() {
                    bail!("material '{}' roughness must be finite", material.name);
                }
                Material::Metal {
                    albedo,
                    roughness: roughness.clamp(0.0, 1.0),
                }
            }
            MaterialData::Dielectric { ior } => {
                if !ior.is_finite() || ior <= 0.0 {
                    bail!(
                        "material '{}' ior must be finite and greater than zero",
                        material.name
                    );
                }
                Material::Dielectric { ior }
            }
            MaterialData::Emissive { color, strength } => {
                validate_color(&material.name, "color", color)?;
                if !strength.is_finite() || strength < 0.0 {
                    bail!(
                        "material '{}' strength must be finite and non-negative",
                        material.name
                    );
                }
                Material::Emissive { color, strength }
            }
        };

        material_names.insert(material.name, materials.len());
        materials.push(runtime_material);
    }

    let mut spheres = Vec::with_capacity(file.objects.len());
    let mut triangles = Vec::with_capacity(file.objects.len());
    let mut triangle_attributes = Vec::with_capacity(file.objects.len());
    let mut textures = Vec::new();
    let mut rigid_bodies = Vec::new();
    for object in file.objects {
        match object {
            ObjectFile::Sphere {
                center,
                radius,
                material,
                group,
                physics: body,
            } => {
                if !center.is_finite() || !radius.is_finite() || radius <= 0.0 {
                    bail!("sphere radius must be positive and its center must be finite");
                }
                let material_index = material_names
                    .get(&material)
                    .copied()
                    .with_context(|| format!("sphere references unknown material '{material}'"))?;
                let group = validate_group(group)?;
                let sphere_index = spheres.len();
                spheres.push(Sphere {
                    center,
                    radius,
                    material_index,
                    group: group.clone(),
                });
                if let Some(body) = body {
                    ensure_physics_block(&physics)?;
                    rigid_bodies.push(validate_rigid_body(
                        body,
                        ColliderKindFile::Sphere,
                        ColliderShape::Sphere { radius },
                        ObjectBinding::Sphere {
                            index: sphere_index,
                        },
                        group,
                    )?);
                }
            }
            ObjectFile::Box {
                center,
                size,
                material,
                group,
                physics: body,
            } => {
                if !center.is_finite() || !size.is_finite() || size.cmple(Vec3::ZERO).any() {
                    bail!("box center must be finite and size components must be positive");
                }
                let material_index = material_names
                    .get(&material)
                    .copied()
                    .with_context(|| format!("box references unknown material '{material}'"))?;
                let group = validate_group(group)?;
                let start = triangles.len();
                for vertices in box_triangles(center, size) {
                    triangles.push(Triangle {
                        vertices,
                        material_index,
                        group: group.clone(),
                    });
                    triangle_attributes.push(TriangleAttributes::default());
                }
                if let Some(body) = body {
                    ensure_physics_block(&physics)?;
                    rigid_bodies.push(validate_rigid_body(
                        body,
                        ColliderKindFile::Cuboid,
                        ColliderShape::Cuboid {
                            half_extents: size * 0.5,
                        },
                        ObjectBinding::Triangles {
                            start,
                            count: 12,
                            pivot: center,
                        },
                        group,
                    )?);
                }
            }
            ObjectFile::Triangle {
                vertices,
                material,
                group,
                physics,
            } => {
                if physics.is_some() {
                    bail!("triangle objects do not support physics in the rigid-body MVP");
                }
                validate_triangle(vertices)?;
                let material_index = material_names.get(&material).copied().with_context(|| {
                    format!("triangle references unknown material '{material}'")
                })?;
                triangles.push(Triangle {
                    vertices,
                    material_index,
                    group: validate_group(group)?,
                });
                triangle_attributes.push(TriangleAttributes::default());
            }
            ObjectFile::Mesh {
                path,
                material,
                group,
                physics,
            } => {
                if physics.is_some() {
                    bail!("mesh objects do not support physics in the rigid-body MVP");
                }
                let material_override = material
                    .as_ref()
                    .map(|material| {
                        material_names.get(material).copied().with_context(|| {
                            format!("mesh references unknown material '{material}'")
                        })
                    })
                    .transpose()?;
                let group = validate_group(group)?;
                let mesh_path = if path.is_absolute() {
                    path
                } else {
                    asset_root.join(path)
                };
                let mesh = toaster_assets::load_gltf(&mesh_path).with_context(|| {
                    format!("failed to load mesh object {}", mesh_path.display())
                })?;

                let imported_materials = if material_override.is_none() {
                    let texture_base = textures.len();
                    for texture in &mesh.textures {
                        textures.push(Texture::new(
                            texture.width,
                            texture.height,
                            texture.rgba8.clone(),
                        )?);
                    }
                    mesh.materials
                        .iter()
                        .map(|material| {
                            let albedo = material.base_color_factor.truncate();
                            match material.base_color_texture {
                                Some(texture_index) => {
                                    let texture_index = texture_base + texture_index;
                                    if texture_index >= textures.len() {
                                        bail!("glTF material references an unknown texture");
                                    }
                                    materials.push(Material::TexturedDiffuse {
                                        albedo,
                                        texture_index,
                                    });
                                }
                                None => materials.push(Material::Diffuse { albedo }),
                            }
                            Ok(materials.len() - 1)
                        })
                        .collect::<Result<Vec<_>>>()?
                } else {
                    Vec::new()
                };
                let default_material = if material_override.is_none()
                    && mesh
                        .triangles
                        .iter()
                        .any(|triangle| triangle.material_index.is_none())
                {
                    materials.push(Material::Diffuse { albedo: Vec3::ONE });
                    Some(materials.len() - 1)
                } else {
                    None
                };

                for (triangle_index, (mesh_triangle, vertices)) in mesh
                    .triangles
                    .iter()
                    .zip(mesh.triangle_vertices())
                    .enumerate()
                {
                    let positions = vertices.map(|vertex| vertex.position);
                    validate_triangle(positions).with_context(|| {
                        format!(
                            "mesh {} contains invalid triangle {}",
                            mesh_path.display(),
                            triangle_index
                        )
                    })?;
                    let normals = collect_options(vertices.map(|vertex| vertex.normal));
                    let tex_coords = collect_options(vertices.map(|vertex| vertex.tex_coord));
                    let material_index = match material_override {
                        Some(material_index) => material_index,
                        None => match mesh_triangle.material_index {
                            Some(material_index) => *imported_materials
                                .get(material_index)
                                .context("glTF primitive references an unknown material")?,
                            None => default_material
                                .expect("a default glTF material was created when required"),
                        },
                    };
                    if matches!(materials[material_index], Material::TexturedDiffuse { .. })
                        && tex_coords.is_none()
                    {
                        bail!(
                            "mesh {} triangle {} uses a base-color texture without TEXCOORD_0",
                            mesh_path.display(),
                            triangle_index
                        );
                    }
                    triangles.push(Triangle {
                        vertices: positions,
                        material_index,
                        group: group.clone(),
                    });
                    triangle_attributes.push(TriangleAttributes {
                        normals,
                        tex_coords,
                    });
                }
            }
        }
    }

    let (background, environment) = match file.render.background {
        BackgroundFile::Kind(BackgroundKindFile::Sky) => (Background::Sky, None),
        BackgroundFile::Kind(BackgroundKindFile::Black) => (Background::Black, None),
        BackgroundFile::Detailed(DetailedBackgroundFile::Environment {
            path,
            intensity,
            rotation_degrees,
        }) => {
            let environment_path = if path.is_absolute() {
                path
            } else {
                asset_root.join(path)
            };
            let environment = EnvironmentMap::load(&environment_path, intensity, rotation_degrees)
                .with_context(|| {
                    format!(
                        "failed to load environment background {}",
                        environment_path.display()
                    )
                })?;
            (Background::Environment, Some(environment))
        }
    };

    let mut animation = file.animation;
    animation.normalize_rotation_axes()?;
    let scene = Scene {
        camera: CameraSettings {
            position: file.camera.position,
            look_at: file.camera.look_at,
            up: file.camera.up,
            fov_degrees: file.camera.fov_degrees,
        },
        render: RenderSettings {
            width: file.render.width,
            height: file.render.height,
            samples: file.render.samples,
            max_bounces: file.render.max_bounces,
            background,
        },
        materials,
        spheres,
        triangles,
        triangle_attributes,
        textures,
        environment,
        animation,
        physics,
        rigid_bodies,
    };
    validate_animation(&scene)?;
    Ok(scene)
}

/// Validates and converts backend-neutral top-level physics controls.
fn validate_physics_settings(file: PhysicsFile) -> Result<PhysicsSettings> {
    if !file.gravity.is_finite() {
        bail!("physics gravity must be finite");
    }
    if !file.timestep.is_finite() || file.timestep <= 0.0 {
        bail!("physics timestep must be finite and greater than zero");
    }
    if !(1..=64).contains(&file.substeps) {
        bail!("physics substeps must be between 1 and 64");
    }
    Ok(PhysicsSettings {
        enabled: file.enabled,
        physics_type: match file.physics_type {
            PhysicsTypeFile::RigidBody => PhysicsType::RigidBody,
        },
        gravity: file.gravity,
        timestep: file.timestep,
        substeps: file.substeps,
    })
}

/// Prevents per-object declarations from silently doing nothing without a world block.
fn ensure_physics_block(physics: &Option<PhysicsSettings>) -> Result<()> {
    if physics.is_none() {
        bail!("object physics declarations require a top-level physics block");
    }
    Ok(())
}

/// Validates one raw body and binds it to already-expanded render geometry.
fn validate_rigid_body(
    file: RigidBodyFile,
    expected_collider: ColliderKindFile,
    collider: ColliderShape,
    binding: ObjectBinding,
    group: Option<String>,
) -> Result<RigidBodyDeclaration> {
    if file
        .collider
        .is_some_and(|collider| collider != expected_collider)
    {
        bail!(
            "physics collider must be '{}' for this object",
            match expected_collider {
                ColliderKindFile::Sphere => "sphere",
                ColliderKindFile::Cuboid => "cuboid",
            }
        );
    }
    if !file.friction.is_finite() || file.friction < 0.0 {
        bail!("physics friction must be finite and non-negative");
    }
    if !file.restitution.is_finite() || !(0.0..=1.0).contains(&file.restitution) {
        bail!("physics restitution must be finite and between 0 and 1");
    }
    if file
        .initial_velocity
        .is_some_and(|velocity| !velocity.is_finite())
        || file
            .initial_angular_velocity
            .is_some_and(|velocity| !velocity.is_finite())
    {
        bail!("physics initial velocities must be finite");
    }

    let (body, mass, initial_velocity, initial_angular_velocity) = match file.body {
        RigidBodyKindFile::Static => {
            if file.mass.is_some()
                || file.initial_velocity.is_some()
                || file.initial_angular_velocity.is_some()
            {
                bail!("static physics bodies do not accept mass or initial velocities");
            }
            (RigidBodyKind::Static, None, Vec3::ZERO, Vec3::ZERO)
        }
        RigidBodyKindFile::Dynamic => {
            let mass = file.mass.unwrap_or(1.0);
            if !mass.is_finite() || mass <= 0.0 {
                bail!("dynamic physics body mass must be finite and greater than zero");
            }
            (
                RigidBodyKind::Dynamic,
                Some(mass),
                file.initial_velocity.unwrap_or(Vec3::ZERO),
                file.initial_angular_velocity.unwrap_or(Vec3::ZERO),
            )
        }
    };

    Ok(RigidBodyDeclaration {
        body,
        collider,
        mass,
        friction: file.friction,
        restitution: file.restitution,
        initial_velocity,
        initial_angular_velocity,
        binding,
        group,
    })
}

/// Expands an axis-aligned box into a stable 12-triangle face ordering.
fn box_triangles(center: Vec3, size: Vec3) -> [[Vec3; 3]; 12] {
    let half = size * 0.5;
    let vertices = [
        center + Vec3::new(-half.x, -half.y, -half.z),
        center + Vec3::new(-half.x, -half.y, half.z),
        center + Vec3::new(-half.x, half.y, -half.z),
        center + Vec3::new(-half.x, half.y, half.z),
        center + Vec3::new(half.x, -half.y, -half.z),
        center + Vec3::new(half.x, -half.y, half.z),
        center + Vec3::new(half.x, half.y, -half.z),
        center + Vec3::new(half.x, half.y, half.z),
    ];
    const INDICES: [[usize; 3]; 12] = [
        [1, 5, 7],
        [1, 7, 3],
        [4, 0, 2],
        [4, 2, 6],
        [0, 1, 3],
        [0, 3, 2],
        [5, 4, 6],
        [5, 6, 7],
        [3, 7, 6],
        [3, 6, 2],
        [0, 4, 5],
        [0, 5, 1],
    ];
    INDICES.map(|face| face.map(|index| vertices[index]))
}

/// Returns three values only when every optional vertex attribute is present.
fn collect_options<T: Copy>(values: [Option<T>; 3]) -> Option<[T; 3]> {
    Some([values[0]?, values[1]?, values[2]?])
}

/// Rejects empty group names while preserving absent groups.
fn validate_group(group: Option<String>) -> Result<Option<String>> {
    if group.as_deref().is_some_and(str::is_empty) {
        bail!("object group name must not be empty");
    }
    Ok(group)
}

/// Ensures triangle vertices are finite and non-collinear.
fn validate_triangle(vertices: [Vec3; 3]) -> Result<()> {
    if vertices.iter().any(|vertex| !vertex.is_finite()) {
        bail!("triangle vertices must be finite");
    }
    let edges = [vertices[1] - vertices[0], vertices[2] - vertices[0]];
    if edges[0].cross(edges[1]).length_squared() <= f32::EPSILON {
        bail!("triangle vertices must not be collinear");
    }
    Ok(())
}

/// Validates a named material's albedo as display-bounded linear RGB.
fn validate_albedo(name: &str, albedo: Vec3) -> Result<()> {
    validate_color(name, "albedo", albedo)
}

/// Validates a named RGB field as finite with every channel in `[0, 1]`.
fn validate_color(name: &str, field: &str, color: Vec3) -> Result<()> {
    if !color.is_finite() || color.cmplt(Vec3::ZERO).any() || color.cmpgt(Vec3::ONE).any() {
        bail!("material '{name}' {field} must be between 0 and 1");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Scene> {
        build_scene(serde_json::from_str(text)?, Path::new("."))
    }

    #[test]
    fn accepts_canonical_fields_and_defaults_up() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,0,1],"look_at":[0,0,0],"fov_degrees":60},
            "render":{"width":4,"height":2,"samples":2,"max_bounces":1},
            "materials":[{"name":"red","type":"diffuse","albedo":[1,0,0]}],
            "objects":[{"type":"sphere","center":[0,0,0],"radius":0.5,"material":"red"}]
        }"#,
        )
        .unwrap();
        assert_eq!(scene.render.samples, 2);
        assert_eq!(scene.camera.up, Vec3::Y);
        assert_eq!(scene.spheres[0].material_index, 0);
    }

    #[test]
    fn accepts_legacy_field_aliases() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,0,1],"look_at":[0,0,0],"vertical_fov_degrees":45},
            "render":{"width":1,"height":1,"samples_per_pixel":1,"max_bounces":1},
            "materials":[],"objects":[]
        }"#,
        )
        .unwrap();
        assert_eq!(scene.camera.fov_degrees, 45.0);
    }

    #[test]
    fn accepts_all_material_types_and_clamps_roughness() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,0,1],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "materials":[
                {"name":"matte","type":"diffuse","albedo":[0.2,0.3,0.4]},
                {"name":"metal","type":"metal","albedo":[0.8,0.8,0.8],"roughness":2.0},
                {"name":"glass","type":"dielectric","ior":1.5}
            ],
            "objects":[]
        }"#,
        )
        .unwrap();

        assert_eq!(
            scene.materials[0],
            Material::Diffuse {
                albedo: Vec3::new(0.2, 0.3, 0.4)
            }
        );
        assert_eq!(
            scene.materials[1],
            Material::Metal {
                albedo: Vec3::splat(0.8),
                roughness: 1.0
            }
        );
        assert_eq!(scene.materials[2], Material::Dielectric { ior: 1.5 });
    }

    #[test]
    fn accepts_emissive_material_triangle_and_black_background() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,0,1],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1,"background":"black"},
            "materials":[{"name":"light","type":"emissive","color":[1,0.8,0.6],"strength":5}],
            "objects":[{
                "type":"triangle",
                "vertices":[[-1,-1,0],[1,-1,0],[0,1,0]],
                "material":"light"
            }]
        }"#,
        )
        .unwrap();

        assert_eq!(scene.render.background, Background::Black);
        assert_eq!(scene.triangles.len(), 1);
        assert_eq!(scene.triangles[0].material_index, 0);
        assert_eq!(
            scene.materials[0],
            Material::Emissive {
                color: Vec3::new(1.0, 0.8, 0.6),
                strength: 5.0
            }
        );
    }

    #[test]
    fn rejects_invalid_material_parameters() {
        for material in [
            r#"{"name":"bad","type":"diffuse","albedo":[1.1,0,0]}"#,
            r#"{"name":"bad","type":"dielectric","ior":0}"#,
        ] {
            let json = format!(
                r#"{{
                "camera":{{"position":[0,0,1],"look_at":[0,0,0],"fov_degrees":45}},
                "render":{{"width":1,"height":1,"samples":1,"max_bounces":1}},
                "materials":[{material}],"objects":[]
            }}"#
            );
            assert!(parse(&json).is_err());
        }
    }

    #[test]
    fn rejects_unknown_materials() {
        let error = parse(
            r#"{
            "camera":{"position":[0,0,1],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "materials":[],
            "objects":[{"type":"sphere","center":[0,0,0],"radius":1,"material":"missing"}]
        }"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown material"));
    }

    #[test]
    fn parses_grouped_animation_tracks() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,0,2],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "materials":[{"name":"red","type":"diffuse","albedo":[1,0,0]}],
            "objects":[{"type":"sphere","group":"ball","center":[1,0,0],"radius":0.5,"material":"red"}],
            "animation":{"tracks":[
                {"type":"translation","target":{"type":"group","name":"ball"},"interpolation":"step","keyframes":[{"time":0,"value":[0,1,0]}]},
                {"type":"rotation","target":{"type":"camera"},"axis":[0,2,0],"pivot":[0,0,0],"interpolation":"linear","keyframes":[{"time":0,"degrees":0},{"time":1,"degrees":90}]}
            ]}
        }"#,
        )
        .unwrap();

        assert_eq!(scene.spheres[0].group.as_deref(), Some("ball"));
        assert_eq!(scene.animation.tracks.len(), 2);
        let crate::AnimationTrack::Rotation { axis, .. } = &scene.animation.tracks[1] else {
            panic!("expected rotation track");
        };
        assert_eq!(*axis, Vec3::Y);
    }

    #[test]
    fn rejects_bad_animation_definitions() {
        for track in [
            r#"{"type":"rotation","target":{"type":"camera"},"axis":[0,0,0],"pivot":[0,0,0],"interpolation":"linear","keyframes":[{"time":0,"degrees":0}]}"#,
            r#"{"type":"translation","target":{"type":"group","name":"missing"},"interpolation":"linear","keyframes":[{"time":0,"value":[0,0,0]}]}"#,
            r#"{"type":"translation","target":{"type":"camera"},"interpolation":"linear","keyframes":[{"time":1,"value":[0,0,0]},{"time":1,"value":[1,0,0]}]}"#,
        ] {
            let json = format!(
                r#"{{
                "camera":{{"position":[0,0,2],"look_at":[0,0,0],"fov_degrees":45}},
                "render":{{"width":1,"height":1,"samples":1,"max_bounces":1}},
                "materials":[],"objects":[],"animation":{{"tracks":[{track}]}}
            }}"#
            );
            assert!(parse(&json).is_err());
        }
    }

    #[test]
    fn loads_rotating_cube_demo() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/006_rotating_cube.json");
        let scene = load_scene(path).unwrap();

        assert_eq!(scene.triangles.len(), 12);
        assert!(scene
            .triangles
            .iter()
            .all(|triangle| triangle.group.as_deref() == Some("cube")));
        assert_eq!(scene.animation.tracks.len(), 1);
    }

    #[test]
    fn loads_relative_gltf_mesh_with_material_and_group() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/008_gltf_tetrahedron.json");
        let scene = load_scene(path).unwrap();

        assert_eq!(scene.triangles.len(), 6);
        let imported = &scene.triangles[2..];
        assert!(imported.iter().all(|triangle| triangle.material_index == 1));
        assert!(imported
            .iter()
            .all(|triangle| triangle.group.as_deref() == Some("tetrahedron")));
        assert_eq!(imported[0].vertices[0], Vec3::new(0.0, 0.75, -2.0));

        let animated = scene.evaluate_at(0.5).unwrap();
        assert_ne!(animated.triangles[2].vertices, scene.triangles[2].vertices);
    }

    #[test]
    fn imports_gltf_base_color_texture_when_material_override_is_omitted() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/009_gltf_textured_quad.json");
        let scene = load_scene(path).unwrap();

        assert_eq!(scene.triangles.len(), 4);
        assert_eq!(scene.triangle_attributes.len(), 4);
        assert!(scene.triangle_attributes[0].normals.is_some());
        assert!(scene.triangle_attributes[0].tex_coords.is_some());
        assert_eq!(scene.textures.len(), 1);
        assert_eq!(scene.materials.len(), 2);
        assert_eq!(
            scene.materials[1],
            Material::TexturedDiffuse {
                albedo: Vec3::new(0.8, 1.0, 0.6),
                texture_index: 0,
            }
        );
        assert!(scene.triangles[..2]
            .iter()
            .all(|triangle| triangle.material_index == 1));

        let animated = scene.evaluate_at(2.0).unwrap();
        let rotated_normal = animated.triangle_attributes[0].normals.unwrap()[0];
        assert!(rotated_normal.abs_diff_eq(Vec3::X, 1e-5));
    }

    #[test]
    fn loads_relative_hdr_environment_background() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/010_environment_map.json");
        let scene = load_scene(path).unwrap();
        let environment = scene.environment.as_ref().unwrap();

        assert_eq!(scene.render.background, Background::Environment);
        assert_eq!((environment.width, environment.height), (4, 2));
        assert_eq!(environment.intensity, 8.0);
        assert_eq!(environment.rotation_degrees, 20.0);
        assert_eq!(environment.pixels.len(), 8);
        assert!(environment
            .pixels
            .iter()
            .any(|pixel| pixel.max_element() > 0.0));
    }

    #[test]
    fn loads_generated_benchmark_suite_with_expected_geometry_counts() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let fixtures = [
            ("001_triangles_128.json", 128, 1, false),
            ("002_triangles_2048.json", 2_048, 1, false),
            ("003_triangles_8192.json", 8_192, 1, false),
            ("004_spheres_64.json", 2, 65, false),
            ("005_spheres_512.json", 2, 513, false),
            ("006_mixed_2048t_128s.json", 2_048, 129, false),
            ("007_environment_control.json", 0, 4, true),
        ];

        for (name, triangles, spheres, has_environment) in fixtures {
            let scene = load_scene(repository_root.join("scenes/benchmarks").join(name)).unwrap();
            assert_eq!(scene.triangles.len(), triangles, "{name}");
            assert_eq!(scene.triangle_attributes.len(), triangles, "{name}");
            assert_eq!(scene.spheres.len(), spheres, "{name}");
            assert_eq!(scene.environment.is_some(), has_environment, "{name}");
            assert!(scene.animation.tracks.is_empty(), "{name}");
            assert_eq!((scene.render.width, scene.render.height), (320, 180));
            assert_eq!((scene.render.samples, scene.render.max_bounces), (4, 4));
        }
    }

    #[test]
    fn parses_rigid_body_sphere_and_expands_box() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,2,5],"look_at":[0,1,0],"fov_degrees":45},
            "render":{"width":4,"height":4,"samples":1,"max_bounces":1},
            "physics":{"type":"rigid_body"},
            "materials":[{"name":"matte","type":"diffuse","albedo":[0.5,0.5,0.5]}],
            "objects":[
                {"type":"sphere","center":[0,3,0],"radius":0.5,"material":"matte","group":"ball",
                 "physics":{"body":"dynamic","collider":"sphere","mass":2,"restitution":0.4}},
                {"type":"box","center":[0,-0.25,0],"size":[4,0.5,4],"material":"matte","group":"floor",
                 "physics":{"body":"static","collider":"cuboid","friction":0.8}}
            ]
        }"#,
        )
        .unwrap();

        assert!(scene.physics.unwrap().enabled);
        assert_eq!(scene.spheres.len(), 1);
        assert_eq!(scene.triangles.len(), 12);
        assert_eq!(scene.triangle_attributes.len(), 12);
        assert_eq!(scene.rigid_bodies.len(), 2);
        assert_eq!(scene.rigid_bodies[0].mass, Some(2.0));
        assert!(scene.triangles.iter().all(|triangle| {
            triangle.group.as_deref() == Some("floor") && triangle.material_index == 0
        }));
        let ObjectBinding::Triangles { count, pivot, .. } = scene.rigid_bodies[1].binding else {
            panic!("expected box triangle binding");
        };
        assert_eq!(count, 12);
        assert_eq!(pivot, Vec3::new(0.0, -0.25, 0.0));
    }

    #[test]
    fn validates_physics_even_when_disabled_but_skips_ownership() {
        let valid = parse(
            r#"{
            "camera":{"position":[0,0,3],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "physics":{"enabled":false,"type":"rigid_body"},
            "materials":[{"name":"m","type":"diffuse","albedo":[1,1,1]}],
            "objects":[{"type":"sphere","center":[0,0,0],"radius":1,"material":"m","group":"hero",
                        "physics":{"body":"dynamic","mass":1}}],
            "animation":{"tracks":[{"type":"translation","target":{"type":"group","name":"hero"},
              "interpolation":"linear","keyframes":[{"time":0,"value":[0,0,0]},{"time":1,"value":[1,0,0]}]}]}
        }"#,
        )
        .unwrap();
        assert!(!valid.physics.unwrap().enabled);
        assert_eq!(valid.evaluate_at(1.0).unwrap().spheres[0].center, Vec3::X);

        let invalid = r#"{
            "camera":{"position":[0,0,3],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "physics":{"enabled":false,"type":"rigid_body"},
            "materials":[{"name":"m","type":"diffuse","albedo":[1,1,1]}],
            "objects":[{"type":"sphere","center":[0,0,0],"radius":1,"material":"m",
                        "physics":{"body":"dynamic","mass":0}}]
        }"#;
        assert!(parse(invalid).is_err());
    }

    #[test]
    fn rejects_enabled_physics_animation_conflicts_and_invalid_bindings() {
        let conflict = r#"{
            "camera":{"position":[0,0,3],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "physics":{"type":"rigid_body"},
            "materials":[{"name":"m","type":"diffuse","albedo":[1,1,1]}],
            "objects":[{"type":"sphere","center":[0,0,0],"radius":1,"material":"m","group":"hero",
                        "physics":{"body":"dynamic"}}],
            "animation":{"tracks":[{"type":"translation","target":{"type":"group","name":"hero"},
              "interpolation":"linear","keyframes":[{"time":0,"value":[0,0,0]}]}]}
        }"#;
        assert!(parse(conflict)
            .unwrap_err()
            .to_string()
            .contains("enabled physics body"));
        assert!(parse(&conflict.replace("\"dynamic\"", "\"static\""))
            .unwrap_err()
            .to_string()
            .contains("enabled physics body"));

        let without_world = r#"{
            "camera":{"position":[0,0,3],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "materials":[{"name":"m","type":"diffuse","albedo":[1,1,1]}],
            "objects":[{"type":"sphere","center":[0,0,0],"radius":1,"material":"m",
                        "physics":{"body":"dynamic"}}]
        }"#;
        assert!(parse(without_world).is_err());

        let mismatch = r#"{
            "camera":{"position":[0,0,3],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "physics":{"type":"rigid_body"},
            "materials":[{"name":"m","type":"diffuse","albedo":[1,1,1]}],
            "objects":[{"type":"box","center":[0,0,0],"size":[1,1,1],"material":"m",
                        "physics":{"body":"dynamic","collider":"sphere"}}]
        }"#;
        assert!(parse(mismatch).is_err());
    }

    #[test]
    fn rejects_invalid_physics_world_and_body_parameters() {
        fn scene_json(physics: &str, object: &str) -> String {
            format!(
                r#"{{
                    "camera":{{"position":[0,0,3],"look_at":[0,0,0],"fov_degrees":45}},
                    "render":{{"width":1,"height":1,"samples":1,"max_bounces":1}},
                    "physics":{physics},
                    "materials":[{{"name":"m","type":"diffuse","albedo":[1,1,1]}}],
                    "objects":[{object}]
                }}"#
            )
        }

        let ordinary_sphere = r#"{"type":"sphere","center":[0,0,0],"radius":1,"material":"m"}"#;
        for physics in [
            r#"{"type":"rigid_body","gravity":[0,1e400,0]}"#,
            r#"{"type":"rigid_body","timestep":0}"#,
            r#"{"type":"rigid_body","substeps":0}"#,
            r#"{"type":"rigid_body","substeps":65}"#,
            r#"{"type":"rigid_body","unknown":true}"#,
        ] {
            assert!(parse(&scene_json(physics, ordinary_sphere)).is_err());
        }

        let invalid_bodies = [
            r#"{"type":"sphere","center":[0,0,0],"radius":1,"material":"m","physics":{"body":"dynamic","mass":0}}"#,
            r#"{"type":"sphere","center":[0,0,0],"radius":1,"material":"m","physics":{"body":"dynamic","friction":-0.1}}"#,
            r#"{"type":"sphere","center":[0,0,0],"radius":1,"material":"m","physics":{"body":"dynamic","restitution":1.1}}"#,
            r#"{"type":"sphere","center":[0,0,0],"radius":1,"material":"m","physics":{"body":"dynamic","initial_velocity":[0,1e400,0]}}"#,
            r#"{"type":"sphere","center":[0,0,0],"radius":1,"material":"m","physics":{"body":"static","mass":1}}"#,
            r#"{"type":"sphere","center":[0,0,0],"radius":1,"material":"m","physics":{"body":"static","initial_angular_velocity":[0,1,0]}}"#,
            r#"{"type":"sphere","center":[0,0,0],"radius":1,"material":"m","physics":{"body":"dynamic","collider":"cuboid"}}"#,
            r#"{"type":"sphere","center":[0,0,0],"radius":1,"material":"m","physics":{"body":"dynamic","unknown":true}}"#,
            r#"{"type":"triangle","vertices":[[-1,0,0],[1,0,0],[0,1,0]],"material":"m","physics":{"body":"static"}}"#,
            r#"{"type":"box","center":[0,0,0],"size":[1,0,1],"material":"m","physics":{"body":"dynamic"}}"#,
        ];
        for object in invalid_bodies {
            assert!(parse(&scene_json(r#"{"type":"rigid_body"}"#, object)).is_err());
        }
    }
}
