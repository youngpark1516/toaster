//! glTF triangle geometry import.

use crate::{Mesh, MeshMaterial, MeshTexture, MeshTriangle, MeshVertex};
use anyhow::{bail, Context, Result};
use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};
use std::path::Path;

/// Loads triangle primitives from the default glTF scene, applying each node's
/// complete world transform. If the document has no default scene, all scenes
/// are loaded in declaration order.
pub fn load_gltf(path: impl AsRef<Path>) -> Result<Mesh> {
    let path = path.as_ref();
    let (document, buffers, images) =
        gltf::import(path).with_context(|| format!("failed to import glTF {}", path.display()))?;
    let mut mesh = Mesh {
        materials: document
            .materials()
            .map(|material| {
                let pbr = material.pbr_metallic_roughness();
                MeshMaterial {
                    base_color_factor: Vec4::from(pbr.base_color_factor()),
                    base_color_texture: pbr.base_color_texture().map(|info| info.texture().index()),
                }
            })
            .collect(),
        textures: document
            .textures()
            .map(|texture| image_to_rgba8(&images[texture.source().index()]))
            .collect::<Result<Vec<_>>>()?,
        ..Mesh::default()
    };

    if let Some(scene) = document.default_scene() {
        load_scene_nodes(scene.nodes(), &buffers, &mut mesh)?;
    } else {
        for scene in document.scenes() {
            load_scene_nodes(scene.nodes(), &buffers, &mut mesh)?;
        }
    }

    if mesh.triangles.is_empty() {
        bail!("glTF {} contains no triangle primitives", path.display());
    }
    Ok(mesh)
}

/// Traverses each root node in a glTF scene with an identity parent transform.
fn load_scene_nodes<'a>(
    nodes: impl Iterator<Item = gltf::Node<'a>>,
    buffers: &[gltf::buffer::Data],
    mesh: &mut Mesh,
) -> Result<()> {
    for node in nodes {
        load_node(node, Mat4::IDENTITY, buffers, mesh)?;
    }
    Ok(())
}

/// Recursively imports one node, composing its transform with all ancestors.
///
/// Triangle primitives append transformed vertices and rebased indices to
/// `output`; unsupported modes or malformed attributes return an error.
fn load_node(
    node: gltf::Node<'_>,
    parent_transform: Mat4,
    buffers: &[gltf::buffer::Data],
    output: &mut Mesh,
) -> Result<()> {
    let local_transform = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world_transform = parent_transform * local_transform;

    if let Some(node_mesh) = node.mesh() {
        for (primitive_index, primitive) in node_mesh.primitives().enumerate() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                bail!(
                    "glTF mesh {} primitive {} uses unsupported mode {:?}; only triangles are supported",
                    node_mesh.index(),
                    primitive_index,
                    primitive.mode()
                );
            }

            let reader = primitive.reader(|buffer| Some(buffers[buffer.index()].0.as_slice()));
            let positions = reader.read_positions().with_context(|| {
                format!(
                    "glTF mesh {} primitive {} has no POSITION attribute",
                    node_mesh.index(),
                    primitive_index
                )
            })?;
            let primitive_positions = positions
                .map(|position| world_transform.transform_point3(Vec3::from(position)))
                .collect::<Vec<_>>();

            if primitive_positions
                .iter()
                .any(|position| !position.is_finite())
            {
                bail!(
                    "glTF mesh {} primitive {} produces non-finite positions",
                    node_mesh.index(),
                    primitive_index
                );
            }

            let primitive_normals = reader
                .read_normals()
                .map(|normals| normals.map(Vec3::from).collect::<Vec<_>>());
            let primitive_tex_coords = reader
                .read_tex_coords(0)
                .map(|tex_coords| tex_coords.into_f32().map(Vec2::from).collect::<Vec<_>>());
            if primitive_normals
                .as_ref()
                .is_some_and(|normals| normals.len() != primitive_positions.len())
                || primitive_tex_coords
                    .as_ref()
                    .is_some_and(|tex_coords| tex_coords.len() != primitive_positions.len())
            {
                bail!(
                    "glTF mesh {} primitive {} has mismatched vertex attribute counts",
                    node_mesh.index(),
                    primitive_index
                );
            }

            let normal_transform = primitive_normals
                .as_ref()
                .map(|_| Mat3::from_mat4(world_transform).inverse().transpose());
            let vertex_count = primitive_positions.len();
            let base_vertex =
                u32::try_from(output.vertices.len()).context("glTF mesh has too many vertices")?;
            let indices = match reader.read_indices() {
                Some(indices) => indices.into_u32().collect::<Vec<_>>(),
                None => (0..u32::try_from(vertex_count).context("glTF primitive is too large")?)
                    .collect(),
            };
            if indices.len() % 3 != 0 {
                bail!(
                    "glTF mesh {} primitive {} has an index count that is not divisible by three",
                    node_mesh.index(),
                    primitive_index
                );
            }
            if indices.iter().any(|&index| index as usize >= vertex_count) {
                bail!(
                    "glTF mesh {} primitive {} contains an out-of-bounds index",
                    node_mesh.index(),
                    primitive_index
                );
            }

            for (vertex_index, position) in primitive_positions.into_iter().enumerate() {
                let normal = primitive_normals.as_ref().map(|normals| {
                    normal_transform.expect("normal transform exists") * normals[vertex_index]
                });
                let normal = normal.map(|normal| normal.normalize_or_zero());
                if normal.is_some_and(|normal| !normal.is_finite() || normal == Vec3::ZERO) {
                    bail!(
                        "glTF mesh {} primitive {} produces an invalid normal",
                        node_mesh.index(),
                        primitive_index
                    );
                }
                let tex_coord = primitive_tex_coords
                    .as_ref()
                    .map(|tex_coords| tex_coords[vertex_index]);
                if tex_coord.is_some_and(|tex_coord| !tex_coord.is_finite()) {
                    bail!(
                        "glTF mesh {} primitive {} contains non-finite texture coordinates",
                        node_mesh.index(),
                        primitive_index
                    );
                }
                output.vertices.push(MeshVertex {
                    position,
                    normal,
                    tex_coord,
                });
            }
            for triangle in indices.chunks_exact(3) {
                output.triangles.push(MeshTriangle {
                    indices: [
                        base_vertex + triangle[0],
                        base_vertex + triangle[1],
                        base_vertex + triangle[2],
                    ],
                    material_index: primitive.material().index(),
                });
            }
        }
    }

    for child in node.children() {
        load_node(child, world_transform, buffers, output)?;
    }
    Ok(())
}

/// Converts supported 8-bit glTF image layouts to tightly packed RGBA8.
fn image_to_rgba8(image: &gltf::image::Data) -> Result<MeshTexture> {
    use gltf::image::Format;

    let pixel_count = image.width as usize * image.height as usize;
    let mut rgba8 = Vec::with_capacity(pixel_count * 4);
    match image.format {
        Format::R8 => {
            for &value in &image.pixels {
                rgba8.extend_from_slice(&[value, value, value, 255]);
            }
        }
        Format::R8G8 => {
            for pixel in image.pixels.chunks_exact(2) {
                rgba8.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        Format::R8G8B8 => {
            for pixel in image.pixels.chunks_exact(3) {
                rgba8.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        Format::R8G8B8A8 => rgba8.extend_from_slice(&image.pixels),
        format => bail!("unsupported glTF base-color image format {format:?}; use an 8-bit image"),
    }
    if rgba8.len() != pixel_count * 4 {
        bail!("glTF image pixel data does not match its dimensions");
    }

    Ok(MeshTexture {
        width: image.width,
        height: image.height,
        rgba8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_indexed_triangles_and_applies_node_transform() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models/tetrahedron.gltf");
        let mesh = load_gltf(path).unwrap();

        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.triangles.len(), 4);
        assert_eq!(mesh.triangles[0].indices, [0, 1, 2]);
        assert_eq!(mesh.vertices[0].position, Vec3::new(0.0, 0.75, -2.0));
        assert_eq!(mesh.vertices[1].position, Vec3::new(-0.75, -0.75, -1.25));
        assert!(mesh.vertices.iter().all(|vertex| vertex.normal.is_none()));
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.tex_coord.is_none()));
        assert!(mesh.materials.is_empty());
        assert!(mesh.textures.is_empty());
    }

    #[test]
    fn loads_normals_uvs_base_color_material_and_texture() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models/textured_quad.gltf");
        let mesh = load_gltf(path).unwrap();

        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.triangles.len(), 2);
        assert_eq!(mesh.triangles[0].material_index, Some(0));
        assert_eq!(mesh.vertices[0].normal, Some(Vec3::Z));
        assert_eq!(mesh.vertices[0].tex_coord, Some(Vec2::new(0.0, 1.0)));
        assert_eq!(
            mesh.materials[0].base_color_factor,
            Vec4::new(0.8, 1.0, 0.6, 1.0)
        );
        assert_eq!(mesh.materials[0].base_color_texture, Some(0));
        assert_eq!((mesh.textures[0].width, mesh.textures[0].height), (2, 2));
        assert_eq!(mesh.textures[0].rgba8.len(), 16);
    }
}
