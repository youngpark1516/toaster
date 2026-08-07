//! Runtime geometric primitives.

use glam::{Vec2, Vec3};

#[derive(Clone, Debug)]
/// A sphere primitive.
pub struct Sphere {
    /// World-space center.
    pub center: Vec3,
    /// Positive world-space radius.
    pub radius: f32,
    /// Index into [`crate::Scene::materials`].
    pub material_index: usize,
    /// Optional animation group shared with other objects.
    pub group: Option<String>,
}

#[derive(Clone, Debug)]
/// A double-sided triangle primitive.
pub struct Triangle {
    /// World-space vertices in winding order.
    pub vertices: [Vec3; 3],
    /// Index into [`crate::Scene::materials`].
    pub material_index: usize,
    /// Optional animation group shared with other objects.
    pub group: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
/// Optional per-vertex shading attributes parallel to one [`Triangle`].
pub struct TriangleAttributes {
    /// Optional smooth normals corresponding to the triangle vertices.
    pub normals: Option<[Vec3; 3]>,
    /// Optional first-set texture coordinates corresponding to the vertices.
    pub tex_coords: Option<[Vec2; 3]>,
}
