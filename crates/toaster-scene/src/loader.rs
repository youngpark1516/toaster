//! JSON scene deserialization, relative asset resolution, and validation.

use crate::{
    animation::{validate_animation, Animation},
    environment::EnvironmentMap,
    material::Material,
    object::{Sphere, Triangle, TriangleAttributes},
    physics::{
        ColliderShape, CollisionPhase, EventReactionDeclaration, MaterialFlashDeclaration,
        ObjectBinding, PhysicsEntityId, PhysicsEventMatcher, PhysicsSettings, PhysicsType,
        ResetBodyDeclaration, RigidBodyDeclaration, RigidBodyKind, SpawnPointDeclaration,
        TeleportBodyDeclaration, TeleportVelocity, TriggerDeclaration, TriggerPhase,
    },
    scene::{Background, CameraSettings, RenderSettings, Scene},
    texture::Texture,
};
use anyhow::{bail, Context, Result};
use glam::{Quat, Vec3};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
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
    /// Optional invisible fixed trigger sensors.
    #[serde(default)]
    triggers: Vec<TriggerFile>,
    /// Optional named absolute destinations for teleport actions.
    #[serde(default)]
    spawn_points: Vec<SpawnPointFile>,
    /// Optional renderer-neutral event response declarations.
    #[serde(default)]
    event_reactions: Vec<EventReactionFile>,
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
        /// Optional stable scene identity; required when physics is present.
        id: Option<String>,
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
        /// Optional stable scene identity; required when physics is present.
        id: Option<String>,
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
        /// Optional stable identity reserved for future scene hooks.
        id: Option<String>,
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
        /// Optional stable identity; required when physics is present.
        id: Option<String>,
        /// Absolute path or path relative to the scene file.
        path: PathBuf,
        /// Optional Toaster material overriding imported base colors.
        material: Option<String>,
        /// Optional animation group assigned to all imported triangles.
        group: Option<String>,
        /// Optional renderer-neutral rigid-body declaration using a proxy collider.
        physics: Option<RigidBodyFile>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
/// Raw invisible trigger-zone declarations.
enum TriggerFile {
    /// Fixed spherical sensor.
    Sphere {
        /// Stable identity shared with physics objects.
        id: String,
        /// World-space center.
        center: Vec3,
        /// Positive radius.
        radius: f32,
    },
    /// Fixed axis-aligned box sensor.
    Box {
        /// Stable identity shared with physics objects.
        id: String,
        /// World-space center.
        center: Vec3,
        /// Positive full extents.
        size: Vec3,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Raw declarative response to one physics-event matcher.
struct EventReactionFile {
    /// Tagged collision or trigger selector.
    #[serde(rename = "match")]
    event_matcher: EventMatcherFile,
    /// Optional temporary material override.
    flash: Option<MaterialFlashFile>,
    /// Optional counter name incremented by each matching event.
    counter: Option<String>,
    /// Optional authored-state reset action.
    reset: Option<ResetBodyFile>,
    /// Optional named-spawn teleport action.
    teleport: Option<TeleportBodyFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Raw named teleport destination.
struct SpawnPointFile {
    /// Nonempty scene-local destination name.
    name: String,
    /// Absolute world-space rigid-body origin.
    position: Vec3,
    /// Optional absolute axis-angle orientation.
    rotation: Option<SpawnRotationFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Raw absolute axis-angle orientation for a spawn point.
struct SpawnRotationFile {
    /// Finite nonzero world-space axis.
    axis: Vec3,
    /// Finite angle in degrees.
    degrees: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Raw reset action.
struct ResetBodyFile {
    /// Concrete dynamic physics body ID.
    target: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Raw teleport action.
struct TeleportBodyFile {
    /// Concrete dynamic physics body ID.
    target: String,
    /// Existing named spawn point.
    spawn: String,
    /// Clear or preserve both linear and angular velocity.
    #[serde(default)]
    velocity: TeleportVelocityFile,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Serialized teleport velocity policy.
enum TeleportVelocityFile {
    /// Zero both velocities.
    #[default]
    Clear,
    /// Retain both velocities.
    Preserve,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
/// Raw physics-event selector with `"*"` and omitted fields as wildcards.
enum EventMatcherFile {
    /// Unordered physical-contact selector.
    Collision {
        /// Required contact lifecycle phase.
        phase: CollisionPhaseFile,
        /// Optional concrete participant or `"*"`.
        object: Option<String>,
        /// Optional second concrete participant or `"*"`.
        other: Option<String>,
    },
    /// Trigger-overlap selector.
    Trigger {
        /// Required sensor lifecycle phase.
        phase: TriggerPhaseFile,
        /// Optional concrete trigger or `"*"`.
        trigger: Option<String>,
        /// Optional concrete moving body or `"*"`.
        object: Option<String>,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Collision phases accepted by event reaction rules.
enum CollisionPhaseFile {
    /// Contact began.
    Started,
    /// Contact persisted for one nominal fixed tick.
    Stayed,
    /// Contact ended.
    Exited,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Trigger phases accepted by event reaction rules.
enum TriggerPhaseFile {
    /// A body entered a sensor.
    Entered,
    /// A body left a sensor.
    Exited,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Raw material-flash action.
struct MaterialFlashFile {
    /// Concrete physics body ID.
    target: String,
    /// Existing named non-emissive material.
    material: String,
    /// Positive flash lifetime in scene seconds.
    duration: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Raw per-object rigid-body properties.
struct RigidBodyFile {
    /// Static or dynamic ownership.
    body: RigidBodyKindFile,
    /// Optional explicit shape assertion.
    collider: Option<ColliderKindFile>,
    /// Required simple proxy shape for imported mesh visuals.
    collider_proxy: Option<ColliderProxyFile>,
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

#[derive(Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
/// Simple collider geometry authored independently from an imported mesh.
enum ColliderProxyFile {
    /// Axis-aligned box proxy with full extents.
    Box {
        /// Absolute world-space proxy center.
        center: Vec3,
        /// Positive full extents.
        size: Vec3,
    },
    /// Spherical proxy.
    Sphere {
        /// Absolute world-space proxy center.
        center: Vec3,
        /// Positive radius.
        radius: f32,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Serialized rigid-body kinds.
enum RigidBodyKindFile {
    /// Immovable body.
    Static,
    /// Simulated body.
    Dynamic,
    /// Animation-driven moving collider.
    Kinematic,
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
    if !file.triggers.is_empty() {
        ensure_physics_block(&physics)?;
    }
    if !file.spawn_points.is_empty() {
        ensure_physics_block(&physics)?;
    }
    if !file.event_reactions.is_empty() {
        ensure_physics_block(&physics)?;
    }

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
    let mut entity_ids = HashSet::new();
    for object in file.objects {
        match object {
            ObjectFile::Sphere {
                id,
                center,
                radius,
                material,
                group,
                physics: body,
            } => {
                let id = validate_object_id(id, body.is_some(), &mut entity_ids)?;
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
                        id.expect("physics object id was required and validated"),
                        ColliderKindFile::Sphere,
                        ColliderShape::Sphere { radius },
                        center,
                        ObjectBinding::Sphere {
                            index: sphere_index,
                        },
                        group,
                    )?);
                }
            }
            ObjectFile::Box {
                id,
                center,
                size,
                material,
                group,
                physics: body,
            } => {
                let id = validate_object_id(id, body.is_some(), &mut entity_ids)?;
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
                        id.expect("physics object id was required and validated"),
                        ColliderKindFile::Cuboid,
                        ColliderShape::Cuboid {
                            half_extents: size * 0.5,
                        },
                        center,
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
                id,
                vertices,
                material,
                group,
                physics,
            } => {
                validate_object_id(id, false, &mut entity_ids)?;
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
                id,
                path,
                material,
                group,
                physics: body,
            } => {
                let id = validate_object_id(id, body.is_some(), &mut entity_ids)?;
                let material_override = material
                    .as_ref()
                    .map(|material| {
                        material_names.get(material).copied().with_context(|| {
                            format!("mesh references unknown material '{material}'")
                        })
                    })
                    .transpose()?;
                let group = validate_group(group)?;
                let start = triangles.len();
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
                if let Some(body) = body {
                    ensure_physics_block(&physics)?;
                    let count = triangles.len() - start;
                    if count == 0 {
                        bail!("physics mesh must import at least one triangle");
                    }
                    rigid_bodies.push(validate_mesh_rigid_body(
                        body,
                        id.expect("physics mesh id was required and validated"),
                        start,
                        count,
                        group,
                    )?);
                }
            }
        }
    }

    let triggers = file
        .triggers
        .into_iter()
        .map(|trigger| validate_trigger(trigger, &mut entity_ids))
        .collect::<Result<Vec<_>>>()?;

    let mut spawn_names = HashSet::new();
    let spawn_points = file
        .spawn_points
        .into_iter()
        .map(|spawn| validate_spawn_point(spawn, &mut spawn_names))
        .collect::<Result<Vec<_>>>()?;

    let reaction_context = EventReactionValidationContext {
        rigid_bodies: &rigid_bodies,
        triggers: &triggers,
        material_names: &material_names,
        materials: &materials,
        spheres: &spheres,
        triangles: &triangles,
        spawn_points: &spawn_points,
    };
    let event_reactions = file
        .event_reactions
        .into_iter()
        .map(|reaction| validate_event_reaction(reaction, &reaction_context))
        .collect::<Result<Vec<_>>>()?;

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
        triggers,
        spawn_points,
        event_reactions,
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
        bail!("physics declarations require a top-level physics block");
    }
    Ok(())
}

/// Validates one raw body and binds it to already-expanded render geometry.
fn validate_rigid_body(
    file: RigidBodyFile,
    id: PhysicsEntityId,
    expected_collider: ColliderKindFile,
    collider: ColliderShape,
    authored_origin: Vec3,
    binding: ObjectBinding,
    group: Option<String>,
) -> Result<RigidBodyDeclaration> {
    if file.collider_proxy.is_some() {
        bail!("sphere and box objects do not accept physics collider_proxy");
    }
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
    finish_rigid_body(file, id, collider, authored_origin, binding, group)
}

/// Validates a mesh proxy and binds the complete imported triangle range to it.
fn validate_mesh_rigid_body(
    mut file: RigidBodyFile,
    id: PhysicsEntityId,
    start: usize,
    count: usize,
    group: Option<String>,
) -> Result<RigidBodyDeclaration> {
    if file.collider.is_some() {
        bail!("mesh physics uses collider_proxy instead of the collider field");
    }
    let proxy = file
        .collider_proxy
        .take()
        .context("mesh physics requires a collider_proxy")?;
    let (collider, center) = match proxy {
        ColliderProxyFile::Box { center, size } => {
            if !center.is_finite() || !size.is_finite() || size.cmple(Vec3::ZERO).any() {
                bail!("mesh box proxy center must be finite and size components must be positive");
            }
            (
                ColliderShape::Cuboid {
                    half_extents: size * 0.5,
                },
                center,
            )
        }
        ColliderProxyFile::Sphere { center, radius } => {
            if !center.is_finite() || !radius.is_finite() || radius <= 0.0 {
                bail!("mesh sphere proxy radius must be positive and its center must be finite");
            }
            (ColliderShape::Sphere { radius }, center)
        }
    };
    finish_rigid_body(
        file,
        id,
        collider,
        center,
        ObjectBinding::Triangles {
            start,
            count,
            pivot: center,
        },
        group,
    )
}

/// Validates body properties shared by primitives and imported visual meshes.
fn finish_rigid_body(
    file: RigidBodyFile,
    id: PhysicsEntityId,
    collider: ColliderShape,
    authored_origin: Vec3,
    binding: ObjectBinding,
    group: Option<String>,
) -> Result<RigidBodyDeclaration> {
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
        RigidBodyKindFile::Kinematic => {
            if group.is_none() {
                bail!("kinematic physics bodies require a nonempty animation group");
            }
            if file.mass.is_some()
                || file.initial_velocity.is_some()
                || file.initial_angular_velocity.is_some()
            {
                bail!("kinematic physics bodies do not accept mass or initial velocities");
            }
            (RigidBodyKind::Kinematic, None, Vec3::ZERO, Vec3::ZERO)
        }
    };

    Ok(RigidBodyDeclaration {
        id,
        body,
        collider,
        mass,
        friction: file.friction,
        restitution: file.restitution,
        initial_velocity,
        initial_angular_velocity,
        initial_transform: crate::RigidTransform {
            translation: authored_origin,
            rotation: Quat::IDENTITY,
        },
        binding,
        group,
    })
}

/// Validates an optional source-object identity and reserves its global name.
fn validate_object_id(
    id: Option<String>,
    required_for_physics: bool,
    entity_ids: &mut HashSet<String>,
) -> Result<Option<PhysicsEntityId>> {
    let Some(id) = id else {
        if required_for_physics {
            bail!("physics objects require a nonempty object id");
        }
        return Ok(None);
    };
    let id = PhysicsEntityId::new(id)?;
    if !entity_ids.insert(id.as_str().to_owned()) {
        bail!("duplicate physics entity id '{}'", id.as_str());
    }
    Ok(Some(id))
}

/// Validates one static invisible trigger declaration.
fn validate_trigger(
    file: TriggerFile,
    entity_ids: &mut HashSet<String>,
) -> Result<TriggerDeclaration> {
    let (id, center, collider) = match file {
        TriggerFile::Sphere { id, center, radius } => {
            if !center.is_finite() || !radius.is_finite() || radius <= 0.0 {
                bail!("trigger sphere radius must be positive and its center must be finite");
            }
            (id, center, ColliderShape::Sphere { radius })
        }
        TriggerFile::Box { id, center, size } => {
            if !center.is_finite() || !size.is_finite() || size.cmple(Vec3::ZERO).any() {
                bail!("trigger box center must be finite and size components must be positive");
            }
            (
                id,
                center,
                ColliderShape::Cuboid {
                    half_extents: size * 0.5,
                },
            )
        }
    };
    let id = PhysicsEntityId::new(id)?;
    if !entity_ids.insert(id.as_str().to_owned()) {
        bail!("duplicate physics entity id '{}'", id.as_str());
    }
    Ok(TriggerDeclaration {
        id,
        center,
        collider,
    })
}

/// Validates one named absolute teleport destination.
fn validate_spawn_point(
    file: SpawnPointFile,
    names: &mut HashSet<String>,
) -> Result<SpawnPointDeclaration> {
    if file.name.is_empty() {
        bail!("spawn point name must not be empty");
    }
    if !names.insert(file.name.clone()) {
        bail!("duplicate spawn point name '{}'", file.name);
    }
    if !file.position.is_finite() {
        bail!("spawn point '{}' position must be finite", file.name);
    }
    let rotation = match file.rotation {
        Some(rotation) => {
            if !rotation.axis.is_finite() || rotation.axis.length_squared() <= f32::EPSILON {
                bail!(
                    "spawn point '{}' rotation axis must be finite and nonzero",
                    file.name
                );
            }
            if !rotation.degrees.is_finite() {
                bail!(
                    "spawn point '{}' rotation degrees must be finite",
                    file.name
                );
            }
            Quat::from_axis_angle(rotation.axis.normalize(), rotation.degrees.to_radians())
        }
        None => Quat::IDENTITY,
    };
    Ok(SpawnPointDeclaration {
        name: file.name,
        transform: crate::RigidTransform {
            translation: file.position,
            rotation,
        },
    })
}

/// Read-only scene data needed to validate event response declarations.
struct EventReactionValidationContext<'a> {
    rigid_bodies: &'a [RigidBodyDeclaration],
    triggers: &'a [TriggerDeclaration],
    material_names: &'a HashMap<String, usize>,
    materials: &'a [Material],
    spheres: &'a [Sphere],
    triangles: &'a [Triangle],
    spawn_points: &'a [SpawnPointDeclaration],
}

/// Validates one backend-neutral event response after all IDs and bindings exist.
fn validate_event_reaction(
    file: EventReactionFile,
    context: &EventReactionValidationContext<'_>,
) -> Result<EventReactionDeclaration> {
    if file.flash.is_none()
        && file.counter.is_none()
        && file.reset.is_none()
        && file.teleport.is_none()
    {
        bail!("event reaction must declare a flash, counter, reset, or teleport action");
    }
    if file.reset.is_some() && file.teleport.is_some() {
        bail!("event reaction cannot declare both reset and teleport actions");
    }
    let counter = file
        .counter
        .map(|counter| {
            if counter.is_empty() {
                bail!("event reaction counter name must not be empty");
            }
            Ok(counter)
        })
        .transpose()?;

    let event_matcher = match file.event_matcher {
        EventMatcherFile::Collision {
            phase,
            object,
            other,
        } => {
            let object = validate_reaction_body_filter(object, context.rigid_bodies, false)?;
            let other = validate_reaction_body_filter(other, context.rigid_bodies, false)?;
            if object.is_some() && object == other {
                bail!("collision reaction cannot match the same body twice");
            }
            PhysicsEventMatcher::Collision {
                phase: match phase {
                    CollisionPhaseFile::Started => CollisionPhase::Started,
                    CollisionPhaseFile::Stayed => CollisionPhase::Stayed,
                    CollisionPhaseFile::Exited => CollisionPhase::Exited,
                },
                object,
                other,
            }
        }
        EventMatcherFile::Trigger {
            phase,
            trigger,
            object,
        } => {
            let trigger = validate_trigger_filter(trigger, context.triggers)?;
            let object = validate_reaction_body_filter(object, context.rigid_bodies, true)?;
            PhysicsEventMatcher::Trigger {
                phase: match phase {
                    TriggerPhaseFile::Entered => TriggerPhase::Entered,
                    TriggerPhaseFile::Exited => TriggerPhase::Exited,
                },
                trigger,
                object,
            }
        }
    };

    let flash = file
        .flash
        .map(|flash| {
            if flash.target == "*" || flash.target.is_empty() {
                bail!("material flash target must be a concrete physics entity id");
            }
            if !flash.duration.is_finite() || flash.duration <= 0.0 {
                bail!("material flash duration must be finite and greater than zero");
            }
            let target = PhysicsEntityId::new(flash.target)?;
            let target_is_explicit = match &event_matcher {
                PhysicsEventMatcher::Collision { object, other, .. } => {
                    object.as_ref() == Some(&target) || other.as_ref() == Some(&target)
                }
                PhysicsEventMatcher::Trigger { object, .. } => object.as_ref() == Some(&target),
            };
            if !target_is_explicit {
                bail!(
                    "material flash target '{}' must be a concrete object in its event matcher",
                    target
                );
            }
            let body = context
                .rigid_bodies
                .iter()
                .find(|body| body.id == target)
                .context("material flash target is not a rendered physics body")?;
            let authored_materials =
                binding_material_indices(&body.binding, context.spheres, context.triangles)?;
            if authored_materials
                .into_iter()
                .any(|material| is_emissive_material(context.materials, material))
            {
                bail!("material flash targets must use non-emissive authored materials");
            }
            let material_index = context
                .material_names
                .get(&flash.material)
                .copied()
                .with_context(|| {
                    format!(
                        "material flash references unknown material '{}'",
                        flash.material
                    )
                })?;
            if is_emissive_material(context.materials, material_index) {
                bail!("material flash materials must be non-emissive");
            }
            Ok(MaterialFlashDeclaration {
                target,
                material_index,
                duration_seconds: flash.duration,
            })
        })
        .transpose()?;

    let reset = file
        .reset
        .map(|reset| -> Result<ResetBodyDeclaration> {
            let target = validate_motion_action_target(
                reset.target,
                &event_matcher,
                context.rigid_bodies,
                "reset",
            )?;
            Ok(ResetBodyDeclaration { target })
        })
        .transpose()?;

    let teleport = file
        .teleport
        .map(|teleport| -> Result<TeleportBodyDeclaration> {
            let target = validate_motion_action_target(
                teleport.target,
                &event_matcher,
                context.rigid_bodies,
                "teleport",
            )?;
            let spawn_point_index = context
                .spawn_points
                .iter()
                .position(|spawn| spawn.name == teleport.spawn)
                .with_context(|| {
                    format!(
                        "teleport action references unknown spawn point '{}'",
                        teleport.spawn
                    )
                })?;
            Ok(TeleportBodyDeclaration {
                target,
                spawn_point_index,
                velocity: match teleport.velocity {
                    TeleportVelocityFile::Clear => TeleportVelocity::Clear,
                    TeleportVelocityFile::Preserve => TeleportVelocity::Preserve,
                },
            })
        })
        .transpose()?;

    Ok(EventReactionDeclaration {
        event_matcher,
        flash,
        counter,
        reset,
        teleport,
    })
}

/// Validates that a motion action names a concrete matched dynamic body.
fn validate_motion_action_target(
    target: String,
    event_matcher: &PhysicsEventMatcher,
    rigid_bodies: &[RigidBodyDeclaration],
    action_name: &str,
) -> Result<PhysicsEntityId> {
    if target == "*" || target.is_empty() {
        bail!("{action_name} target must be a concrete physics entity id");
    }
    let target = PhysicsEntityId::new(target)?;
    let target_is_explicit = match event_matcher {
        PhysicsEventMatcher::Collision { object, other, .. } => {
            object.as_ref() == Some(&target) || other.as_ref() == Some(&target)
        }
        PhysicsEventMatcher::Trigger { object, .. } => object.as_ref() == Some(&target),
    };
    if !target_is_explicit {
        bail!(
            "{action_name} target '{}' must be a concrete object in its event matcher",
            target
        );
    }
    let body = rigid_bodies
        .iter()
        .find(|body| body.id == target)
        .with_context(|| format!("{action_name} target '{target}' is not a physics body"))?;
    if body.body != RigidBodyKind::Dynamic {
        bail!("{action_name} target '{target}' must be a dynamic body");
    }
    Ok(target)
}

/// Converts omitted and `"*"` filters to wildcards and validates concrete bodies.
fn validate_reaction_body_filter(
    filter: Option<String>,
    rigid_bodies: &[RigidBodyDeclaration],
    trigger_participant: bool,
) -> Result<Option<PhysicsEntityId>> {
    let Some(filter) = filter else {
        return Ok(None);
    };
    if filter == "*" {
        return Ok(None);
    }
    let id = PhysicsEntityId::new(filter)?;
    let body = rigid_bodies
        .iter()
        .find(|body| body.id == id)
        .with_context(|| format!("event reaction references unknown physics body '{id}'"))?;
    if trigger_participant && body.body == RigidBodyKind::Static {
        bail!("trigger reaction object '{id}' must be dynamic or kinematic");
    }
    Ok(Some(id))
}

/// Converts an omitted or `"*"` trigger filter to a wildcard.
fn validate_trigger_filter(
    filter: Option<String>,
    triggers: &[TriggerDeclaration],
) -> Result<Option<PhysicsEntityId>> {
    let Some(filter) = filter else {
        return Ok(None);
    };
    if filter == "*" {
        return Ok(None);
    }
    let id = PhysicsEntityId::new(filter)?;
    if !triggers.iter().any(|trigger| trigger.id == id) {
        bail!("event reaction references unknown trigger '{id}'");
    }
    Ok(Some(id))
}

/// Returns every authored material used by a rigid-body render binding.
fn binding_material_indices(
    binding: &ObjectBinding,
    spheres: &[Sphere],
    triangles: &[Triangle],
) -> Result<Vec<usize>> {
    match *binding {
        ObjectBinding::Sphere { index } => spheres
            .get(index)
            .map(|sphere| vec![sphere.material_index])
            .context("material flash sphere binding is out of range"),
        ObjectBinding::Triangles { start, count, .. } => {
            let triangles = triangles
                .get(start..start.saturating_add(count))
                .context("material flash triangle binding is out of range")?;
            if triangles.is_empty() {
                bail!("material flash triangle binding is empty");
            }
            Ok(triangles
                .iter()
                .map(|triangle| triangle.material_index)
                .collect())
        }
    }
}

fn is_emissive_material(materials: &[Material], index: usize) -> bool {
    matches!(materials.get(index), Some(Material::Emissive { .. }))
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
    fn loads_kinematic_platform_demo_with_stable_bindings() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/012_physics_kinematic_platform.json");
        let scene = load_scene(path).unwrap();
        let kinematic = scene
            .rigid_bodies
            .iter()
            .filter(|body| body.body == RigidBodyKind::Kinematic)
            .collect::<Vec<_>>();

        assert_eq!(kinematic.len(), 2);
        assert_eq!(scene.spheres.len(), 4);
        assert_eq!(scene.triangles.len(), 134);
        assert!(kinematic
            .iter()
            .all(|body| matches!(body.binding, ObjectBinding::Triangles { count: 12, .. })));
        assert!(kinematic.iter().all(|body| body.mass.is_none()));
    }

    #[test]
    fn loads_physics_event_demo_with_invisible_triggers() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/013_physics_events_triggers.json");
        let scene = load_scene(path).unwrap();

        assert_eq!(scene.triggers.len(), 2);
        assert_eq!(scene.rigid_bodies.len(), 10);
        assert_eq!(scene.spheres.len(), 6);
        assert_eq!(scene.triangles.len(), 134);
        assert_eq!(scene.triggers[0].id.as_str(), "gate_zone");
        assert_eq!(scene.triggers[1].id.as_str(), "orb_zone");
    }

    #[test]
    fn loads_event_reaction_demo_with_flashes_and_named_counters() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/014_physics_event_reactions.json");
        let scene = load_scene(path).unwrap();

        assert_eq!(scene.triggers.len(), 2);
        assert_eq!(scene.event_reactions.len(), 6);
        assert_eq!(scene.materials.len(), 12);
        assert_eq!(
            scene
                .event_reactions
                .iter()
                .filter(|reaction| reaction.flash.is_some())
                .count(),
            3
        );
        assert_eq!(
            scene
                .event_reactions
                .iter()
                .filter_map(|reaction| reaction.counter.as_deref())
                .collect::<Vec<_>>(),
            vec!["collision_starts", "gate_entries", "orb_entries"]
        );
    }

    #[test]
    fn loads_reset_teleport_demo_with_named_spawn_point() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/015_physics_reset_teleport.json");
        let scene = load_scene(path).unwrap();

        assert_eq!(scene.spawn_points.len(), 1);
        assert_eq!(scene.spawn_points[0].name, "goal_spawn");
        assert_eq!(
            scene
                .event_reactions
                .iter()
                .filter(|reaction| reaction.reset.is_some())
                .count(),
            2
        );
        assert_eq!(
            scene
                .event_reactions
                .iter()
                .filter(|reaction| reaction.teleport.is_some())
                .count(),
            1
        );
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
                {"id":"ball","type":"sphere","center":[0,3,0],"radius":0.5,"material":"matte","group":"ball",
                 "physics":{"body":"dynamic","collider":"sphere","mass":2,"restitution":0.4}},
                {"id":"floor","type":"box","center":[0,-0.25,0],"size":[4,0.5,4],"material":"matte","group":"floor",
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
    fn parses_stable_physics_ids_and_static_trigger_shapes() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,2,5],"look_at":[0,1,0],"fov_degrees":45},
            "render":{"width":4,"height":4,"samples":1,"max_bounces":1},
            "physics":{"enabled":false,"type":"rigid_body"},
            "materials":[{"name":"m","type":"diffuse","albedo":[0.5,0.5,0.5]}],
            "objects":[
              {"id":"ball","type":"sphere","center":[0,2,0],"radius":0.5,"material":"m",
               "physics":{"body":"dynamic"}}
            ],
            "triggers":[
              {"id":"box_zone","shape":"box","center":[0,1,0],"size":[2,4,6]},
              {"id":"sphere_zone","shape":"sphere","center":[2,1,0],"radius":1.25}
            ]
        }"#,
        )
        .unwrap();

        assert_eq!(scene.rigid_bodies[0].id.as_str(), "ball");
        assert_eq!(scene.triggers.len(), 2);
        assert_eq!(scene.triggers[0].id.as_str(), "box_zone");
        assert_eq!(scene.triggers[0].center, Vec3::Y);
        assert_eq!(
            scene.triggers[0].collider,
            ColliderShape::Cuboid {
                half_extents: Vec3::new(1.0, 2.0, 3.0)
            }
        );
        assert_eq!(
            scene.triggers[1].collider,
            ColliderShape::Sphere { radius: 1.25 }
        );
    }

    #[test]
    fn parses_exact_and_wildcard_event_reactions() {
        let scene = parse(
            r#"{
              "camera":{"position":[0,2,5],"look_at":[0,1,0],"fov_degrees":45},
              "render":{"width":4,"height":4,"samples":1,"max_bounces":1},
              "physics":{"enabled":false,"type":"rigid_body"},
              "materials":[
                {"name":"base","type":"diffuse","albedo":[0.5,0.5,0.5]},
                {"name":"flash","type":"diffuse","albedo":[1,1,0]}
              ],
              "objects":[
                {"id":"ball","type":"sphere","center":[0,2,0],"radius":0.5,"material":"base",
                 "physics":{"body":"dynamic"}},
                {"id":"floor","type":"box","center":[0,-0.1,0],"size":[4,0.2,4],"material":"base",
                 "physics":{"body":"static"}}
              ],
              "triggers":[{"id":"zone","shape":"sphere","center":[0,1,0],"radius":1}],
              "event_reactions":[
                {
                  "match":{"type":"collision","phase":"started","object":"ball","other":"floor"},
                  "flash":{"target":"ball","material":"flash","duration":0.2},
                  "counter":"floor_hits"
                },
                {
                  "match":{"type":"trigger","phase":"entered","trigger":"zone","object":"*"},
                  "counter":"zone_entries"
                },
                {
                  "match":{"type":"collision","phase":"stayed","object":"*","other":"*"},
                  "counter":"contact_ticks"
                }
              ]
            }"#,
        )
        .unwrap();

        assert_eq!(scene.event_reactions.len(), 3);
        let first = &scene.event_reactions[0];
        assert_eq!(first.counter.as_deref(), Some("floor_hits"));
        assert_eq!(first.flash.as_ref().unwrap().target.as_str(), "ball");
        assert_eq!(first.flash.as_ref().unwrap().material_index, 1);
        assert!(matches!(
            scene.event_reactions[1].event_matcher,
            PhysicsEventMatcher::Trigger {
                trigger: Some(_),
                object: None,
                ..
            }
        ));
        assert!(matches!(
            scene.event_reactions[2].event_matcher,
            PhysicsEventMatcher::Collision {
                phase: CollisionPhase::Stayed,
                object: None,
                other: None,
            }
        ));
    }

    #[test]
    fn parses_reset_teleport_actions_and_spawn_rotation() {
        let scene = parse(
            r#"{
              "camera":{"position":[0,2,5],"look_at":[0,1,0],"fov_degrees":45},
              "render":{"width":4,"height":4,"samples":1,"max_bounces":1},
              "physics":{"enabled":false,"type":"rigid_body"},
              "materials":[{"name":"m","type":"diffuse","albedo":[0.5,0.5,0.5]}],
              "objects":[
                {"id":"ball","type":"sphere","center":[0,2,0],"radius":0.5,"material":"m",
                 "physics":{"body":"dynamic","initial_velocity":[1,0,0]}},
                {"id":"floor","type":"box","center":[0,-0.1,0],"size":[4,0.2,4],"material":"m",
                 "physics":{"body":"static"}}
              ],
              "triggers":[{"id":"zone","shape":"sphere","center":[0,1,0],"radius":1}],
              "spawn_points":[
                {"name":"goal","position":[2,3,4],"rotation":{"axis":[0,2,0],"degrees":90}},
                {"name":"plain","position":[0,5,0]}
              ],
              "event_reactions":[
                {"match":{"type":"trigger","phase":"entered","trigger":"zone","object":"ball"},
                 "reset":{"target":"ball"},"counter":"resets"},
                {"match":{"type":"collision","phase":"started","object":"ball","other":"floor"},
                 "teleport":{"target":"ball","spawn":"goal","velocity":"preserve"}},
                {"match":{"type":"collision","phase":"exited","object":"ball","other":"floor"},
                 "teleport":{"target":"ball","spawn":"plain"}}
              ]
            }"#,
        )
        .unwrap();

        assert_eq!(scene.spawn_points.len(), 2);
        assert_eq!(scene.spawn_points[0].name, "goal");
        assert!((scene.spawn_points[0].transform.rotation * Vec3::X).abs_diff_eq(-Vec3::Z, 1.0e-5));
        assert_eq!(
            scene.rigid_bodies[0].initial_transform.translation,
            Vec3::new(0.0, 2.0, 0.0)
        );
        assert_eq!(
            scene.rigid_bodies[0].initial_transform.rotation,
            Quat::IDENTITY
        );
        assert!(scene.event_reactions[0].reset.is_some());
        assert_eq!(
            scene.event_reactions[1].teleport.as_ref().unwrap().velocity,
            TeleportVelocity::Preserve
        );
        assert_eq!(
            scene.event_reactions[2].teleport.as_ref().unwrap().velocity,
            TeleportVelocity::Clear
        );
    }

    #[test]
    fn rejects_invalid_motion_actions_and_spawn_points() {
        fn scene(spawn_points: &str, reaction: &str) -> String {
            format!(
                r#"{{
                  "camera":{{"position":[0,2,5],"look_at":[0,1,0],"fov_degrees":45}},
                  "render":{{"width":1,"height":1,"samples":1,"max_bounces":1}},
                  "physics":{{"type":"rigid_body"}},
                  "materials":[{{"name":"m","type":"diffuse","albedo":[0.5,0.5,0.5]}}],
                  "objects":[
                    {{"id":"ball","type":"sphere","center":[0,2,0],"radius":0.5,"material":"m","physics":{{"body":"dynamic"}}}},
                    {{"id":"floor","type":"box","center":[0,-0.1,0],"size":[4,0.2,4],"material":"m","physics":{{"body":"static"}}}}
                  ],
                  "triggers":[{{"id":"zone","shape":"sphere","center":[0,1,0],"radius":1}}],
                  "spawn_points":[{spawn_points}],
                  "event_reactions":[{reaction}]
                }}"#
            )
        }
        let spawn = r#"{"name":"goal","position":[0,3,0]}"#;
        let invalid_reactions = [
            r#"{"match":{"type":"trigger","phase":"entered","trigger":"zone","object":"*"},"reset":{"target":"ball"}}"#,
            r#"{"match":{"type":"collision","phase":"started","object":"ball","other":"floor"},"reset":{"target":"missing"}}"#,
            r#"{"match":{"type":"collision","phase":"started","object":"ball","other":"floor"},"reset":{"target":"floor"}}"#,
            r#"{"match":{"type":"collision","phase":"started","object":"ball","other":"floor"},"teleport":{"target":"ball","spawn":"missing"}}"#,
            r#"{"match":{"type":"collision","phase":"started","object":"ball","other":"floor"},"teleport":{"target":"ball","spawn":"goal","velocity":"invalid"}}"#,
            r#"{"match":{"type":"collision","phase":"started","object":"ball","other":"floor"},"reset":{"target":"ball"},"teleport":{"target":"ball","spawn":"goal"}}"#,
            r#"{"match":{"type":"collision","phase":"started","object":"ball","other":"floor"},"reset":{"target":"ball","unknown":true}}"#,
        ];
        for reaction in invalid_reactions {
            assert!(parse(&scene(spawn, reaction)).is_err(), "{reaction}");
        }

        let reaction = r#"{"match":{"type":"collision","phase":"started","object":"ball","other":"floor"},"reset":{"target":"ball"}}"#;
        for invalid_spawn in [
            r#"{"name":"","position":[0,0,0]}"#,
            r#"{"name":"goal","position":[0,1e400,0]}"#,
            r#"{"name":"goal","position":[0,0,0],"rotation":{"axis":[0,0,0],"degrees":1}}"#,
            r#"{"name":"goal","position":[0,0,0],"rotation":{"axis":[0,1,0],"degrees":1e400}}"#,
            r#"{"name":"goal","position":[0,0,0],"unknown":true}"#,
        ] {
            assert!(
                parse(&scene(invalid_spawn, reaction)).is_err(),
                "{invalid_spawn}"
            );
        }
        assert!(parse(&scene(
            r#"{"name":"goal","position":[0,0,0]},{"name":"goal","position":[1,0,0]}"#,
            reaction
        ))
        .is_err());

        let kinematic_target = r#"{
          "camera":{"position":[0,2,5],"look_at":[0,1,0],"fov_degrees":45},
          "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
          "physics":{"enabled":false,"type":"rigid_body"},
          "materials":[{"name":"m","type":"diffuse","albedo":[0.5,0.5,0.5]}],
          "objects":[
            {"id":"ball","type":"sphere","center":[0,2,0],"radius":0.5,"material":"m","physics":{"body":"dynamic"}},
            {"id":"mover","type":"box","center":[0,0,0],"size":[1,1,1],"material":"m","group":"mover","physics":{"body":"kinematic"}}
          ],
          "event_reactions":[
            {"match":{"type":"collision","phase":"started","object":"ball","other":"mover"},"reset":{"target":"mover"}}
          ]
        }"#;
        assert!(parse(kinematic_target).is_err());

        let without_physics = r#"{
          "camera":{"position":[0,0,3],"look_at":[0,0,0],"fov_degrees":45},
          "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
          "materials":[],"objects":[],
          "spawn_points":[{"name":"goal","position":[0,0,0]}]
        }"#;
        assert!(parse(without_physics).is_err());
    }

    #[test]
    fn rejects_invalid_event_reaction_actions_and_targets() {
        fn scene(reaction: &str, material: &str, body_material: &str) -> String {
            format!(
                r#"{{
                  "camera":{{"position":[0,2,5],"look_at":[0,1,0],"fov_degrees":45}},
                  "render":{{"width":1,"height":1,"samples":1,"max_bounces":1}},
                  "physics":{{"type":"rigid_body"}},
                  "materials":[
                    {{"name":"base","type":"diffuse","albedo":[0.5,0.5,0.5]}},
                    {material}
                  ],
                  "objects":[
                    {{"id":"ball","type":"sphere","center":[0,2,0],"radius":0.5,
                      "material":"{body_material}","physics":{{"body":"dynamic"}}}},
                    {{"id":"floor","type":"box","center":[0,-0.1,0],"size":[4,0.2,4],
                      "material":"base","physics":{{"body":"static"}}}}
                  ],
                  "triggers":[{{"id":"zone","shape":"sphere","center":[0,1,0],"radius":1}}],
                  "event_reactions":[{reaction}]
                }}"#
            )
        }
        let diffuse = r#"{"name":"flash","type":"diffuse","albedo":[1,1,0]}"#;
        let invalid_reactions = [
            r#"{"match":{"type":"collision","phase":"started"}}"#,
            r#"{"match":{"type":"collision","phase":"started","object":"*"},"flash":{"target":"ball","material":"flash","duration":0.2}}"#,
            r#"{"match":{"type":"collision","phase":"started","object":"ball"},"flash":{"target":"*","material":"flash","duration":0.2}}"#,
            r#"{"match":{"type":"collision","phase":"started","object":"missing"},"counter":"hits"}"#,
            r#"{"match":{"type":"trigger","phase":"entered","trigger":"missing"},"counter":"hits"}"#,
            r#"{"match":{"type":"collision","phase":"started"},"counter":""}"#,
            r#"{"match":{"type":"collision","phase":"started","object":"ball"},"flash":{"target":"ball","material":"flash","duration":0}}"#,
            r#"{"match":{"type":"collision","phase":"started"},"counter":"hits","unknown":true}"#,
        ];
        for reaction in invalid_reactions {
            assert!(
                parse(&scene(reaction, diffuse, "base")).is_err(),
                "{reaction}"
            );
        }

        let valid_flash = r#"{"match":{"type":"collision","phase":"started","object":"ball"},"flash":{"target":"ball","material":"flash","duration":0.2}}"#;
        let emissive = r#"{"name":"flash","type":"emissive","color":[1,1,1],"strength":0}"#;
        assert!(parse(&scene(valid_flash, emissive, "base")).is_err());
        let flash_emissive_target = r#"{"match":{"type":"collision","phase":"started","object":"ball"},"flash":{"target":"ball","material":"base","duration":0.2}}"#;
        assert!(parse(&scene(flash_emissive_target, emissive, "flash")).is_err());

        let without_physics = r#"{
          "camera":{"position":[0,0,3],"look_at":[0,0,0],"fov_degrees":45},
          "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
          "materials":[],"objects":[],
          "event_reactions":[{"match":{"type":"collision","phase":"started"},"counter":"hits"}]
        }"#;
        assert!(parse(without_physics).is_err());
    }

    #[test]
    fn rejects_invalid_or_duplicate_event_identities_and_triggers() {
        fn scene_json(physics: &str, object: &str, triggers: &str) -> String {
            format!(
                r#"{{
                  "camera":{{"position":[0,0,4],"look_at":[0,0,0],"fov_degrees":45}},
                  "render":{{"width":1,"height":1,"samples":1,"max_bounces":1}},
                  {physics}
                  "materials":[{{"name":"m","type":"diffuse","albedo":[1,1,1]}}],
                  "objects":[{object}],
                  "triggers":[{triggers}]
                }}"#
            )
        }
        let body_without_id = r#"{"type":"sphere","center":[0,0,0],"radius":1,"material":"m","physics":{"body":"dynamic"}}"#;
        assert!(parse(&scene_json(
            r#""physics":{"type":"rigid_body"},"#,
            body_without_id,
            ""
        ))
        .is_err());

        let body = r#"{"id":"body","type":"sphere","center":[0,0,0],"radius":1,"material":"m","physics":{"body":"dynamic"}}"#;
        let invalid_triggers = [
            r#"{"id":"","shape":"sphere","center":[0,0,0],"radius":1}"#,
            r#"{"id":"zone","shape":"sphere","center":[0,0,0],"radius":0}"#,
            r#"{"id":"zone","shape":"box","center":[0,0,0],"size":[1,0,1]}"#,
            r#"{"id":"zone","shape":"box","center":[0,0,0],"size":[1,1,1],"unknown":true}"#,
        ];
        for trigger in invalid_triggers {
            assert!(parse(&scene_json(
                r#""physics":{"type":"rigid_body"},"#,
                body,
                trigger
            ))
            .is_err());
        }

        let duplicate = r#"{"id":"body","shape":"sphere","center":[0,0,0],"radius":1}"#;
        assert!(parse(&scene_json(
            r#""physics":{"type":"rigid_body"},"#,
            body,
            duplicate
        ))
        .is_err());

        let trigger = r#"{"id":"zone","shape":"sphere","center":[0,0,0],"radius":1}"#;
        assert!(parse(&scene_json("", "", trigger)).is_err());
    }

    #[test]
    fn validates_physics_even_when_disabled_but_skips_ownership() {
        let valid = parse(
            r#"{
            "camera":{"position":[0,0,3],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "physics":{"enabled":false,"type":"rigid_body"},
            "materials":[{"name":"m","type":"diffuse","albedo":[1,1,1]}],
            "objects":[{"id":"hero","type":"sphere","center":[0,0,0],"radius":1,"material":"m","group":"hero",
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
    fn parses_animation_driven_kinematic_sphere_and_box() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,2,5],"look_at":[0,1,0],"fov_degrees":45},
            "render":{"width":4,"height":4,"samples":1,"max_bounces":1},
            "physics":{"type":"rigid_body","substeps":2},
            "materials":[{"name":"m","type":"diffuse","albedo":[0.5,0.5,0.5]}],
            "objects":[
                {"id":"orb","type":"sphere","center":[0,1,0],"radius":0.5,"material":"m","group":"orb",
                 "physics":{"body":"kinematic","collider":"sphere","friction":0.4}},
                {"id":"platform","type":"box","center":[0,0,0],"size":[2,0.2,2],"material":"m","group":"platform",
                 "physics":{"body":"kinematic","collider":"cuboid","restitution":0.1}}
            ],
            "animation":{"tracks":[
              {"type":"translation","target":{"type":"group","name":"orb"},"interpolation":"linear",
               "keyframes":[{"time":0,"value":[0,0,0]},{"time":1,"value":[1,0,0]}]},
              {"type":"rotation","target":{"type":"group","name":"platform"},"axis":[0,1,0],"pivot":[0,0,0],
               "interpolation":"linear","keyframes":[{"time":0,"degrees":0},{"time":1,"degrees":90}]}
            ]}
        }"#,
        )
        .unwrap();

        assert_eq!(scene.rigid_bodies.len(), 2);
        assert!(scene
            .rigid_bodies
            .iter()
            .all(|body| body.body == RigidBodyKind::Kinematic && body.mass.is_none()));
        assert_eq!(scene.triangles.len(), 12);
    }

    #[test]
    fn validates_kinematic_fields_and_enabled_ownership() {
        fn scene_json(enabled: bool, objects: &str, tracks: &str) -> String {
            format!(
                r#"{{
                  "camera":{{"position":[0,0,4],"look_at":[0,0,0],"fov_degrees":45}},
                  "render":{{"width":1,"height":1,"samples":1,"max_bounces":1}},
                  "physics":{{"enabled":{enabled},"type":"rigid_body"}},
                  "materials":[{{"name":"m","type":"diffuse","albedo":[1,1,1]}}],
                  "objects":[{objects}],
                  "animation":{{"tracks":[{tracks}]}}
                }}"#
            )
        }
        let body = r#"{"id":"mover","type":"sphere","center":[0,0,0],"radius":1,"material":"m","group":"mover","physics":{"body":"kinematic"}}"#;
        let track = r#"{"type":"translation","target":{"type":"group","name":"mover"},"interpolation":"linear","keyframes":[{"time":0,"value":[0,0,0]}]}"#;

        assert!(parse(&scene_json(true, body, track)).is_ok());
        assert!(parse(&scene_json(true, body, "")).is_err());
        assert!(parse(&scene_json(false, body, "")).is_ok());

        for field in [
            r#","mass":1"#,
            r#","initial_velocity":[1,0,0]"#,
            r#","initial_angular_velocity":[0,1,0]"#,
        ] {
            let invalid = body.replace(
                r#""physics":{"body":"kinematic"}"#,
                &format!(r#""physics":{{"body":"kinematic"{field}}}"#),
            );
            assert!(parse(&scene_json(false, &invalid, "")).is_err());
        }

        let missing_group = body.replace(r#","group":"mover""#, "");
        assert!(parse(&scene_json(false, &missing_group, "")).is_err());

        let reused = format!(
            "{body},{{\"type\":\"sphere\",\"center\":[2,0,0],\"radius\":0.5,\"material\":\"m\",\"group\":\"mover\"}}"
        );
        assert!(parse(&scene_json(true, &reused, track)).is_err());
        assert!(parse(&scene_json(false, &reused, track)).is_ok());
    }

    #[test]
    fn enabled_physics_allows_camera_and_unrelated_group_animation() {
        let scene = parse(
            r#"{
            "camera":{"position":[0,0,5],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "physics":{"type":"rigid_body"},
            "materials":[{"name":"m","type":"diffuse","albedo":[1,1,1]}],
            "objects":[
              {"id":"body","type":"sphere","center":[0,2,0],"radius":0.5,"material":"m","group":"body",
               "physics":{"body":"dynamic"}},
              {"type":"sphere","center":[-1,0,0],"radius":0.25,"material":"m","group":"prop"}
            ],
            "animation":{"tracks":[
              {"type":"translation","target":{"type":"group","name":"prop"},
               "interpolation":"linear","keyframes":[{"time":0,"value":[0,0,0]},{"time":1,"value":[1,0,0]}]},
              {"type":"rotation","target":{"type":"camera"},"axis":[0,1,0],"pivot":[0,0,0],
               "interpolation":"linear","keyframes":[{"time":0,"degrees":0},{"time":1,"degrees":90}]}
            ]}
        }"#,
        )
        .unwrap();
        let evaluated = scene.evaluate_at(1.0).unwrap();

        assert_eq!(evaluated.spheres[0].center, Vec3::new(0.0, 2.0, 0.0));
        assert_eq!(evaluated.spheres[1].center, Vec3::ZERO);
        assert!(evaluated.camera.position.abs_diff_eq(Vec3::X * 5.0, 1.0e-5));
    }

    #[test]
    fn rejects_enabled_physics_animation_conflicts_and_invalid_bindings() {
        let conflict = r#"{
            "camera":{"position":[0,0,3],"look_at":[0,0,0],"fov_degrees":45},
            "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
            "physics":{"type":"rigid_body"},
            "materials":[{"name":"m","type":"diffuse","albedo":[1,1,1]}],
            "objects":[{"id":"hero","type":"sphere","center":[0,0,0],"radius":1,"material":"m","group":"hero",
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

    #[test]
    fn loads_mesh_bodies_with_box_and_sphere_proxies() {
        let asset_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let scene = build_scene(
            serde_json::from_str(
                r#"{
                    "camera":{"position":[0,0,8],"look_at":[0,1,-3],"fov_degrees":45},
                    "render":{"width":1,"height":1,"samples":1,"max_bounces":1},
                    "physics":{"type":"rigid_body"},
                    "materials":[],
                    "objects":[
                      {"id":"crate","type":"mesh","path":"assets/models/physics_crate.gltf","group":"crate",
                       "physics":{"body":"dynamic","collider_proxy":{"shape":"box","center":[-1.5,3,-3],"size":[1.2,1.2,1.2]}}},
                      {"id":"orb","type":"mesh","path":"assets/models/physics_crate.gltf","group":"orb",
                       "physics":{"body":"dynamic","collider_proxy":{"shape":"sphere","center":[-1.5,3,-3],"radius":0.7}}},
                      {"id":"pedestal","type":"mesh","path":"assets/models/physics_crate.gltf","group":"pedestal",
                       "physics":{"body":"static","collider_proxy":{"shape":"box","center":[-1.5,3,-3],"size":[1.2,1.2,1.2]}}},
                      {"id":"pusher","type":"mesh","path":"assets/models/physics_crate.gltf","group":"pusher",
                       "physics":{"body":"kinematic","collider_proxy":{"shape":"box","center":[-1.5,3,-3],"size":[1.2,1.2,1.2]}}}
                    ],
                    "animation":{"tracks":[{"type":"translation","target":{"type":"group","name":"pusher"},
                      "interpolation":"linear","keyframes":[{"time":0,"value":[0,0,0]},{"time":1,"value":[1,0,0]}]}]}
                }"#,
            )
            .unwrap(),
            &asset_root,
        )
        .unwrap();

        assert_eq!(scene.rigid_bodies.len(), 4);
        assert_eq!(scene.triangles.len(), 48);
        assert!(matches!(
            scene.rigid_bodies[0].collider,
            ColliderShape::Cuboid { half_extents } if half_extents == Vec3::splat(0.6)
        ));
        assert!(matches!(
            scene.rigid_bodies[1].collider,
            ColliderShape::Sphere { radius } if radius == 0.7
        ));
        assert!(matches!(
            scene.rigid_bodies[0].binding,
            ObjectBinding::Triangles { start: 0, count: 12, pivot }
                if pivot == Vec3::new(-1.5, 3.0, -3.0)
        ));
        assert_eq!(
            scene.rigid_bodies[0].initial_transform.rotation,
            Quat::IDENTITY
        );
        assert_eq!(scene.triangles[0].vertices[0], Vec3::new(-2.1, 2.4, -3.6));
        assert_ne!(
            scene.triangles[0].material_index,
            scene.triangles[6].material_index
        );
    }

    #[test]
    fn rejects_invalid_mesh_proxy_contracts_and_gltf_alias() {
        let asset_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let scene_json = |object: &str| {
            format!(
                r#"{{
                    "camera":{{"position":[0,0,8],"look_at":[0,1,-3],"fov_degrees":45}},
                    "render":{{"width":1,"height":1,"samples":1,"max_bounces":1}},
                    "physics":{{"type":"rigid_body"}},
                    "materials":[{{"name":"m","type":"diffuse","albedo":[1,1,1]}}],
                    "objects":[{object}]
                }}"#
            )
        };
        for object in [
            r#"{"id":"m","type":"mesh","path":"assets/models/physics_crate.gltf","physics":{"body":"dynamic"}}"#,
            r#"{"id":"m","type":"mesh","path":"assets/models/physics_crate.gltf","physics":{"body":"dynamic","collider":"cuboid","collider_proxy":{"shape":"box","center":[0,0,0],"size":[1,1,1]}}}"#,
            r#"{"id":"m","type":"mesh","path":"assets/models/physics_crate.gltf","physics":{"body":"dynamic","collider_proxy":{"shape":"box","center":[0,0,0],"size":[1,0,1]}}}"#,
            r#"{"id":"m","type":"mesh","path":"assets/models/physics_crate.gltf","physics":{"body":"dynamic","collider_proxy":{"shape":"sphere","center":[0,0,0],"radius":0}}}"#,
            r#"{"id":"m","type":"mesh","path":"assets/models/physics_crate.gltf","physics":{"body":"dynamic","collider_proxy":{"shape":"sphere","center":[0,0,0],"radius":1,"extra":true}}}"#,
            r#"{"id":"s","type":"sphere","center":[0,0,0],"radius":1,"material":"m","physics":{"body":"dynamic","collider_proxy":{"shape":"sphere","center":[0,0,0],"radius":1}}}"#,
            r#"{"id":"m","type":"gltf","path":"assets/models/physics_crate.gltf"}"#,
        ] {
            let result = serde_json::from_str(&scene_json(object))
                .map_err(anyhow::Error::from)
                .and_then(|file| build_scene(file, &asset_root));
            assert!(result.is_err());
        }
    }

    #[test]
    fn loads_gltf_proxy_demo_with_stable_ranges_and_materials() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/016_physics_gltf_proxies.json");
        let scene = load_scene(path).unwrap();
        let crate_body = scene
            .rigid_bodies
            .iter()
            .find(|body| body.id.as_str() == "visual_crate")
            .unwrap();
        let ObjectBinding::Triangles {
            start,
            count,
            pivot,
        } = crate_body.binding
        else {
            panic!("mesh crate must bind a triangle range");
        };

        assert_eq!(count, 12);
        assert_eq!(pivot, Vec3::new(-1.5, 3.0, -3.0));
        assert!(scene.triangles[start..start + count]
            .iter()
            .all(|triangle| triangle.group.as_deref() == Some("visual_crate")));
        assert!(scene.triangles[start..start + count]
            .windows(2)
            .any(|pair| pair[0].material_index != pair[1].material_index));
        assert!(scene.rigid_bodies.iter().any(
            |body| body.id.as_str() == "terrain_pedestal" && body.body == RigidBodyKind::Static
        ));
        assert!(
            scene
                .rigid_bodies
                .iter()
                .any(|body| body.id.as_str() == "mesh_pusher"
                    && body.body == RigidBodyKind::Kinematic)
        );
    }
}
