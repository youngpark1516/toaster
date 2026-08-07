//! Top-level renderer-neutral scene state.

use crate::{
    animation::Animation,
    environment::EnvironmentMap,
    material::Material,
    object::{Sphere, Triangle, TriangleAttributes},
    physics::{PhysicsSettings, RigidBodyDeclaration},
    texture::Texture,
};
use glam::Vec3;

#[derive(Clone, Debug)]
/// A fully loaded and validated scene consumed by both renderers.
pub struct Scene {
    /// Camera definition at the scene's base pose.
    pub camera: CameraSettings,
    /// Image-quality and background settings.
    pub render: RenderSettings,
    /// Materials referenced by primitive indices.
    pub materials: Vec<Material>,
    /// Sphere primitives.
    pub spheres: Vec<Sphere>,
    /// Triangle primitives, including expanded imported meshes.
    pub triangles: Vec<Triangle>,
    /// Shading attributes parallel to [`Self::triangles`].
    pub triangle_attributes: Vec<TriangleAttributes>,
    /// Base-color textures referenced by textured materials.
    pub textures: Vec<Texture>,
    /// Loaded map when [`RenderSettings::background`] is [`Background::Environment`].
    pub environment: Option<EnvironmentMap>,
    /// Animation tracks evaluated from this immutable base pose.
    pub animation: Animation,
    /// Optional validated physics-world controls.
    pub physics: Option<PhysicsSettings>,
    /// Logical rigid bodies bound to flattened render primitives.
    pub rigid_bodies: Vec<RigidBodyDeclaration>,
}

#[derive(Clone, Copy, Debug)]
/// Pinhole camera input stored independently of either renderer's camera basis.
pub struct CameraSettings {
    /// World-space camera position.
    pub position: Vec3,
    /// World-space point the camera looks toward.
    pub look_at: Vec3,
    /// Approximate world-space up direction.
    pub up: Vec3,
    /// Vertical field of view in degrees, strictly between 0 and 180.
    pub fov_degrees: f32,
}

#[derive(Clone, Copy, Debug)]
/// Image dimensions and path-tracing quality controls.
pub struct RenderSettings {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Samples traced per pixel or independent frame.
    pub samples: u32,
    /// Maximum scattering depth per path.
    pub max_bounces: u32,
    /// Radiance source used when a ray leaves the scene.
    pub background: Background,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// Background radiance mode.
pub enum Background {
    /// Procedural blue-to-white sky gradient.
    #[default]
    Sky,
    /// Zero radiance.
    Black,
    /// Loaded equirectangular environment map.
    Environment,
}
