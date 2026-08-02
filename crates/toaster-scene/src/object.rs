use glam::{Vec2, Vec3};

#[derive(Clone, Debug)]
pub struct Sphere {
    pub center: Vec3,
    pub radius: f32,
    pub material_index: usize,
    pub group: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Triangle {
    pub vertices: [Vec3; 3],
    pub material_index: usize,
    pub group: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TriangleAttributes {
    pub normals: Option<[Vec3; 3]>,
    pub tex_coords: Option<[Vec2; 3]>,
}
