use crate::{
    animation::Animation,
    environment::EnvironmentMap,
    material::Material,
    object::{Sphere, Triangle, TriangleAttributes},
    texture::Texture,
};
use glam::Vec3;

#[derive(Clone, Debug)]
pub struct Scene {
    pub camera: CameraSettings,
    pub render: RenderSettings,
    pub materials: Vec<Material>,
    pub spheres: Vec<Sphere>,
    pub triangles: Vec<Triangle>,
    pub triangle_attributes: Vec<TriangleAttributes>,
    pub textures: Vec<Texture>,
    pub environment: Option<EnvironmentMap>,
    pub animation: Animation,
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
    pub background: Background,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Background {
    #[default]
    Sky,
    Black,
    Environment,
}
