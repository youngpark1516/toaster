//! Reusable absolute transforms over flattened renderer-neutral geometry.

use crate::{ObjectBinding, Scene};
use anyhow::{bail, Result};
use glam::{Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
/// Absolute world-space rigid pose independent of a physics implementation.
pub struct RigidTransform {
    /// World-space object origin.
    pub translation: Vec3,
    /// World-space object orientation.
    pub rotation: Quat,
}

/// Reconstructs one bound object from immutable base geometry at an absolute pose.
pub fn apply_rigid_transform(
    base: &Scene,
    evaluated: &mut Scene,
    binding: &ObjectBinding,
    transform: RigidTransform,
) -> Result<()> {
    if !transform.translation.is_finite() || !transform.rotation.is_finite() {
        bail!("rigid transform must be finite");
    }

    match *binding {
        ObjectBinding::Sphere { index } => {
            let sphere = evaluated
                .spheres
                .get_mut(index)
                .ok_or_else(|| anyhow::anyhow!("physics sphere binding is out of range"))?;
            sphere.center = transform.translation;
        }
        ObjectBinding::Triangles {
            start,
            count,
            pivot,
        } => {
            let end = start
                .checked_add(count)
                .ok_or_else(|| anyhow::anyhow!("physics triangle binding overflows"))?;
            let base_triangles = base
                .triangles
                .get(start..end)
                .ok_or_else(|| anyhow::anyhow!("physics triangle binding is out of range"))?;
            let output_triangles = evaluated
                .triangles
                .get_mut(start..end)
                .ok_or_else(|| anyhow::anyhow!("evaluated triangle binding is out of range"))?;
            let base_attributes = base
                .triangle_attributes
                .get(start..end)
                .ok_or_else(|| anyhow::anyhow!("physics triangle attributes are out of range"))?;
            let output_attributes = evaluated
                .triangle_attributes
                .get_mut(start..end)
                .ok_or_else(|| anyhow::anyhow!("evaluated triangle attributes are out of range"))?;

            for (((source, target), source_attributes), target_attributes) in base_triangles
                .iter()
                .zip(output_triangles)
                .zip(base_attributes)
                .zip(output_attributes)
            {
                for (source_vertex, target_vertex) in
                    source.vertices.iter().zip(&mut target.vertices)
                {
                    *target_vertex =
                        transform.translation + transform.rotation * (*source_vertex - pivot);
                }
                *target_attributes = *source_attributes;
                if let Some(normals) = &mut target_attributes.normals {
                    for normal in normals {
                        *normal = transform.rotation * *normal;
                    }
                }
            }
        }
    }
    Ok(())
}
