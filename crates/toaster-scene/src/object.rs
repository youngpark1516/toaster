use glam::Vec3;

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
