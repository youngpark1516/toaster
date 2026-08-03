//! Renderer-independent asset import and intermediate mesh data.

/// glTF/GLB loading and world-transform application.
pub mod gltf_loader;
/// Indexed mesh, material, texture, and vertex representations.
pub mod mesh;

pub use gltf_loader::load_gltf;
pub use mesh::{Mesh, MeshMaterial, MeshTexture, MeshTriangle, MeshVertex};
