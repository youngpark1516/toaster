//! Flat BVH layout for renderer upload.

use crate::{
    aabb::Aabb,
    bvh::{Bvh, BvhNode, PrimitiveRef},
};

#[derive(Clone, Copy, Debug)]
/// GPU-friendly node whose tag is encoded by `primitive_count`.
pub struct FlatNode {
    /// World-space node bounds.
    pub bounds: Aabb,
    /// First leaf primitive or right-child node index.
    pub first_primitive_or_right_child: u32,
    /// Leaf primitive count; zero identifies an interior node.
    pub primitive_count: u32,
}

impl FlatNode {
    /// Returns whether this node owns a primitive range.
    pub fn is_leaf(self) -> bool {
        self.primitive_count > 0
    }

    /// Returns the first primitive index for a leaf.
    pub fn first_primitive(self) -> u32 {
        debug_assert!(self.is_leaf());
        self.first_primitive_or_right_child
    }

    /// Returns the right-child node index for an interior node.
    pub fn right_child(self) -> u32 {
        debug_assert!(!self.is_leaf());
        self.first_primitive_or_right_child
    }
}

#[derive(Clone, Debug, Default)]
/// Depth-first flat hierarchy and its leaf-ordered primitive references.
pub struct FlatBvh {
    /// Nodes with each left child immediately following its parent.
    pub nodes: Vec<FlatNode>,
    /// Primitive references stored in contiguous leaf ranges.
    pub primitives: Vec<PrimitiveRef>,
}

impl FlatBvh {
    /// Constructs and flattens a hierarchy from primitive build records.
    pub fn build(primitive_info: &mut [crate::bvh::PrimitiveInfo]) -> Self {
        Self::from_bvh(Bvh::build(primitive_info))
    }

    /// Converts a recursive hierarchy into depth-first storage.
    pub fn from_bvh(bvh: Bvh) -> Self {
        let mut nodes = Vec::new();
        if let Some(root) = &bvh.root {
            flatten_node(root, &mut nodes);
        }

        Self {
            nodes,
            primitives: bvh.primitives,
        }
    }

    /// Returns whether the hierarchy contains no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

fn flatten_node(node: &BvhNode, nodes: &mut Vec<FlatNode>) -> u32 {
    let node_index = nodes.len() as u32;
    nodes.push(FlatNode {
        bounds: node.bounds(),
        first_primitive_or_right_child: 0,
        primitive_count: 0,
    });

    match node {
        BvhNode::Leaf {
            first_primitive,
            primitive_count,
            ..
        } => {
            nodes[node_index as usize].first_primitive_or_right_child = *first_primitive;
            nodes[node_index as usize].primitive_count = *primitive_count;
        }
        BvhNode::Interior { left, right, .. } => {
            flatten_node(left, nodes);
            let right_child = flatten_node(right, nodes);
            nodes[node_index as usize].first_primitive_or_right_child = right_child;
        }
    }

    node_index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bvh::{Bvh, BvhNode};
    use glam::Vec3;

    #[test]
    fn flattens_left_child_next_and_right_child_by_index() {
        let bounds = Aabb::new(Vec3::ZERO, Vec3::ONE);
        let bvh = Bvh {
            root: Some(BvhNode::Interior {
                bounds,
                left: Box::new(BvhNode::Leaf {
                    bounds,
                    first_primitive: 0,
                    primitive_count: 1,
                }),
                right: Box::new(BvhNode::Leaf {
                    bounds,
                    first_primitive: 1,
                    primitive_count: 1,
                }),
            }),
            primitives: vec![PrimitiveRef::Sphere(0), PrimitiveRef::Triangle(0)],
        };

        let flat = FlatBvh::from_bvh(bvh);

        assert_eq!(flat.nodes.len(), 3);
        assert!(!flat.nodes[0].is_leaf());
        assert_eq!(flat.nodes[0].right_child(), 2);
        assert!(flat.nodes[1].is_leaf());
        assert!(flat.nodes[2].is_leaf());
    }
}
