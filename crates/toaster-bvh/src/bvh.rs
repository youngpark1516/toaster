//! Bounding-volume hierarchy construction.

use crate::aabb::Aabb;
use glam::Vec3;

pub const DEFAULT_MAX_LEAF_PRIMITIVES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveRef {
    Sphere(u32),
    Triangle(u32),
}

#[derive(Clone, Copy, Debug)]
pub struct PrimitiveInfo {
    pub bounds: Aabb,
    pub centroid: Vec3,
    pub primitive: PrimitiveRef,
}

#[derive(Clone, Debug)]
pub enum BvhNode {
    Interior {
        bounds: Aabb,
        left: Box<BvhNode>,
        right: Box<BvhNode>,
    },
    Leaf {
        bounds: Aabb,
        first_primitive: u32,
        primitive_count: u32,
    },
}

impl BvhNode {
    pub fn bounds(&self) -> Aabb {
        match self {
            Self::Interior { bounds, .. } | Self::Leaf { bounds, .. } => *bounds,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Bvh {
    pub root: Option<BvhNode>,
    pub primitives: Vec<PrimitiveRef>,
}

impl Bvh {
    pub fn build(primitive_info: &mut [PrimitiveInfo]) -> Self {
        Self::build_with_leaf_size(primitive_info, DEFAULT_MAX_LEAF_PRIMITIVES)
    }

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
