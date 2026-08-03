//! In-progress bounding-volume hierarchy integration boundary.
//!
//! The incoming BVH implementation will own CPU construction and a flattened
//! traversal representation. Until that branch lands, the modules below remain
//! intentionally empty contracts and are not used by either renderer.

/// Intended home for finite axis-aligned bounds and ray-box tests.
pub mod aabb;
/// Intended home for hierarchy construction over scene primitives.
pub mod bvh;
/// Intended home for converting constructed nodes to GPU-friendly flat data.
pub mod flatten;
