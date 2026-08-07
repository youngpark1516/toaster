//! Bounding-volume hierarchy construction.

use crate::aabb::Aabb;
use glam::Vec3;

/// Default maximum number of primitives stored in one leaf.
pub const DEFAULT_MAX_LEAF_PRIMITIVES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Stable reference to a primitive in the renderer-neutral scene arrays.
pub enum PrimitiveRef {
    /// Index into the scene sphere array.
    Sphere(u32),
    /// Index into the scene triangle array.
    Triangle(u32),
}

#[derive(Clone, Copy, Debug)]
/// Bounds and centroid used while partitioning one primitive.
pub struct PrimitiveInfo {
    /// World-space primitive bounds.
    pub bounds: Aabb,
    /// Point used to select and sort along a split axis.
    pub centroid: Vec3,
    /// Stable scene primitive reference.
    pub primitive: PrimitiveRef,
}

#[derive(Clone, Debug)]
/// Recursive hierarchy node produced during construction.
pub enum BvhNode {
    /// Node with two non-empty children.
    Interior {
        /// Bounds enclosing both children.
        bounds: Aabb,
        /// Lower partition, flattened immediately after its parent.
        left: Box<BvhNode>,
        /// Upper partition.
        right: Box<BvhNode>,
    },
    /// Node referencing a contiguous range in the ordered primitive array.
    Leaf {
        /// Bounds enclosing the leaf primitives.
        bounds: Aabb,
        /// First index in [`Bvh::primitives`].
        first_primitive: u32,
        /// Number of contiguous primitive references.
        primitive_count: u32,
    },
}

impl BvhNode {
    /// Returns this node's world-space bounds.
    pub fn bounds(&self) -> Aabb {
        match self {
            Self::Interior { bounds, .. } | Self::Leaf { bounds, .. } => *bounds,
        }
    }
}

#[derive(Clone, Debug)]
/// Recursive hierarchy plus primitive references reordered into leaf ranges.
pub struct Bvh {
    /// Root node, or `None` when built from no primitives.
    pub root: Option<BvhNode>,
    /// Primitive references in leaf traversal order.
    pub primitives: Vec<PrimitiveRef>,
}

impl Bvh {
    /// Builds a median-split hierarchy with the default leaf size.
    pub fn build(primitive_info: &mut [PrimitiveInfo]) -> Self {
        Self::build_with_leaf_size(primitive_info, DEFAULT_MAX_LEAF_PRIMITIVES)
    }

    /// Builds a median-split hierarchy with an explicit nonzero leaf limit.
    pub fn build_with_leaf_size(
        primitive_info: &mut [PrimitiveInfo],
        max_leaf_primitives: usize,
    ) -> Self {
        let mut ordered_primitives = Vec::with_capacity(primitive_info.len());
        let max_leaf_primitives = max_leaf_primitives.max(1);
        let root = build_recursive(primitive_info, &mut ordered_primitives, max_leaf_primitives);

        Self {
            root,
            primitives: ordered_primitives,
        }
    }
}

fn build_recursive(
    primitive_info: &mut [PrimitiveInfo],
    ordered_primitives: &mut Vec<PrimitiveRef>,
    max_leaf_primitives: usize,
) -> Option<BvhNode> {
    if primitive_info.is_empty() {
        return None;
    }

    let bounds = primitive_info
        .iter()
        .fold(Aabb::empty(), |bounds, primitive| {
            bounds.union(primitive.bounds)
        });

    if primitive_info.len() <= max_leaf_primitives {
        return Some(create_leaf(bounds, primitive_info, ordered_primitives));
    }

    let centroid_bounds = primitive_info
        .iter()
        .fold(Aabb::empty(), |bounds, primitive| {
            let centroid = primitive.centroid;
            bounds.union(Aabb::new(centroid, centroid))
        });
    let split_axis = centroid_bounds.longest_axis();

    if centroid_bounds.extent()[split_axis] <= f32::EPSILON {
        return Some(create_leaf(bounds, primitive_info, ordered_primitives));
    }

    primitive_info
        .sort_by(|left, right| left.centroid[split_axis].total_cmp(&right.centroid[split_axis]));

    let split_index = primitive_info.len() / 2;
    let (left_info, right_info) = primitive_info.split_at_mut(split_index);

    let left = build_recursive(left_info, ordered_primitives, max_leaf_primitives)
        .expect("left split is non-empty");
    let right = build_recursive(right_info, ordered_primitives, max_leaf_primitives)
        .expect("right split is non-empty");

    Some(BvhNode::Interior {
        bounds,
        left: Box::new(left),
        right: Box::new(right),
    })
}

fn create_leaf(
    bounds: Aabb,
    primitive_info: &[PrimitiveInfo],
    ordered_primitives: &mut Vec<PrimitiveRef>,
) -> BvhNode {
    let first_primitive = ordered_primitives.len() as u32;
    ordered_primitives.extend(primitive_info.iter().map(|primitive| primitive.primitive));

    BvhNode::Leaf {
        bounds,
        first_primitive,
        primitive_count: primitive_info.len() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primitive(index: u32, x: f32) -> PrimitiveInfo {
        let min = Vec3::new(x, 0.0, 0.0);
        let max = Vec3::new(x + 0.5, 1.0, 1.0);
        let bounds = Aabb::new(min, max);
        PrimitiveInfo {
            bounds,
            centroid: bounds.centroid(),
            primitive: PrimitiveRef::Sphere(index),
        }
    }

    #[test]
    fn builds_empty_bvh() {
        let bvh = Bvh::build(&mut []);

        assert!(bvh.root.is_none());
        assert!(bvh.primitives.is_empty());
    }

    #[test]
    fn orders_primitives_into_leaves() {
        let mut primitives = vec![
            primitive(0, 0.0),
            primitive(1, 4.0),
            primitive(2, 2.0),
            primitive(3, 6.0),
        ];
        let bvh = Bvh::build_with_leaf_size(&mut primitives, 1);

        assert!(bvh.root.is_some());
        assert_eq!(bvh.primitives.len(), 4);
        for index in 0..4 {
            assert!(bvh.primitives.contains(&PrimitiveRef::Sphere(index)));
        }
    }

    #[test]
    fn coincident_centroids_create_leaf() {
        let mut primitives = vec![primitive(0, 0.0), primitive(1, 0.0), primitive(2, 0.0)];
        let bvh = Bvh::build_with_leaf_size(&mut primitives, 1);

        let Some(BvhNode::Leaf {
            primitive_count, ..
        }) = bvh.root
        else {
            panic!("expected leaf");
        };
        assert_eq!(primitive_count, 3);
    }
}
