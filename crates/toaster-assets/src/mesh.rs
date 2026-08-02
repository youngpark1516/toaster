//! Renderer-independent indexed triangle mesh data.

use glam::{Vec2, Vec3, Vec4};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshVertex {
    pub position: Vec3,
    pub normal: Option<Vec3>,
    pub tex_coord: Option<Vec2>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshTriangle {
    pub indices: [u32; 3],
    pub material_index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshTexture {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshMaterial {
    pub base_color_factor: Vec4,
    pub base_color_texture: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<MeshVertex>,
    pub triangles: Vec<MeshTriangle>,
    pub materials: Vec<MeshMaterial>,
    pub textures: Vec<MeshTexture>,
}

impl Mesh {
    pub fn triangle_vertices(&self) -> impl Iterator<Item = [MeshVertex; 3]> + '_ {
        self.triangles.iter().map(|triangle| {
            [
                self.vertices[triangle.indices[0] as usize],
                self.vertices[triangle.indices[1] as usize],
                self.vertices[triangle.indices[2] as usize],
            ]
        })
    }
}
