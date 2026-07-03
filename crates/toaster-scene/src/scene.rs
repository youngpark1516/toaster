use crate::{material::Material, object::Sphere};
use glam::Vec3;

#[derive(Clone, Debug)]
pub struct Scene {
    pub camera: CameraSettings,
    pub render: RenderSettings,
    pub materials: Vec<Material>,
    pub spheres: Vec<Sphere>,
}

#[derive(Clone, Copy, Debug)]
pub struct CameraSettings {
    pub position: Vec3,
    pub look_at: Vec3,
    pub up: Vec3,
    pub fov_degrees: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderSettings {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub max_bounces: u32,
}
