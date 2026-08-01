use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Material {
    Diffuse { albedo: Vec3 },
    TexturedDiffuse { albedo: Vec3, texture_index: usize },
    Metal { albedo: Vec3, roughness: f32 },
    Dielectric { ior: f32 },
    Emissive { color: Vec3, strength: f32 },
}
