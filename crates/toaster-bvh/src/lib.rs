//! Bounding-volume hierarchy construction and renderer-friendly flattening.

/// Finite axis-aligned bounds and ray-box tests.
pub mod aabb;
/// Hierarchy construction over stable scene primitive references.
pub mod bvh;
/// Conversion from recursive nodes to CPU/GPU-friendly flat data.
pub mod flatten;

pub use aabb::Aabb;
pub use bvh::{Bvh, BvhNode, PrimitiveInfo, PrimitiveRef};
pub use flatten::{FlatBvh, FlatNode};
