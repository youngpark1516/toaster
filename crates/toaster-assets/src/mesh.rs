//! Renderer-independent indexed triangle mesh data.

use glam::{Vec2, Vec3, Vec4};

#[derive(Clone, Copy, Debug, PartialEq)]
/// One imported mesh vertex with optional shading attributes.
pub struct MeshVertex {
    /// World-space position after glTF node transforms.
    pub position: Vec3,
    /// Optional inverse-transpose-transformed smooth normal.
    pub normal: Option<Vec3>,
    /// Optional first-set texture coordinate.
    pub tex_coord: Option<Vec2>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// An indexed triangle and its optional imported material.
pub struct MeshTriangle {
    /// Indices into [`Mesh::vertices`].
    pub indices: [u32; 3],
    /// Index into [`Mesh::materials`], or `None` for glTF's default material.
    pub material_index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// An imported 8-bit texture normalized to tightly packed RGBA bytes.
pub struct MeshTexture {
    /// Texture width in pixels.
    pub width: u32,
    /// Texture height in pixels.
    pub height: u32,
    /// Row-major RGBA8 texels.
    pub rgba8: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// The subset of glTF material data currently understood by Toaster.
pub struct MeshMaterial {
    /// Linear base-color multiplier from the glTF material.
    pub base_color_factor: Vec4,
    /// Optional index into [`Mesh::textures`].
    pub base_color_texture: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
/// Renderer-neutral imported mesh data.
pub struct Mesh {
    /// Shared vertex array.
    pub vertices: Vec<MeshVertex>,
    /// Indexed triangle array.
    pub triangles: Vec<MeshTriangle>,
    /// Imported base-color material records.
    pub materials: Vec<MeshMaterial>,
    /// Imported base-color textures.
    pub textures: Vec<MeshTexture>,
}

impl Mesh {
    /// Iterates over triangles expanded into three copied vertices.
    ///
    /// # Panics
    ///
    /// Panics if a triangle contains an invalid index. [`crate::load_gltf`]
    /// validates indices before constructing a `Mesh`.
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
