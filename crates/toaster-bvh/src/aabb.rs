//! Axis-aligned bounding boxes.

use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn expand(self, amount: f32) -> Self {
        let delta = Vec3::splat(amount);
        Self {
            min: self.min - delta,
            max: self.max + delta,
        }
    }

    pub fn centroid(self) -> Vec3 {
        0.5 * (self.min + self.max)
    }

    pub fn extent(self) -> Vec3 {
        self.max - self.min
    }

    pub fn surface_area(self) -> f32 {
        let extent = self.extent().max(Vec3::ZERO);
        2.0 * (extent.x * extent.y + extent.x * extent.z + extent.y * extent.z)
    }

    pub fn longest_axis(self) -> usize {
        let extent = self.extent();
        if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        }
    }

    pub fn intersects_ray(
        self,
        origin: Vec3,
        inverse_direction: Vec3,
        min_distance: f32,
        max_distance: f32,
    ) -> bool {
        let mut t_min = min_distance;
        let mut t_max = max_distance;

        for axis in 0..3 {
            let origin_axis = origin[axis];
            let min_axis = self.min[axis];
            let max_axis = self.max[axis];

            if inverse_direction[axis].is_infinite() {
                if origin_axis < min_axis || origin_axis > max_axis {
                    return false;
                }
                continue;
            }

            let mut near = (min_axis - origin_axis) * inverse_direction[axis];
            let mut far = (max_axis - origin_axis) * inverse_direction[axis];
            if near > far {
                std::mem::swap(&mut near, &mut far);
            }

            t_min = t_min.max(near);
            t_max = t_max.min(far);
            if t_max < t_min {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unions_boxes() {
        let first = Aabb::new(Vec3::new(-1.0, 0.0, 2.0), Vec3::new(1.0, 2.0, 3.0));
        let second = Aabb::new(Vec3::new(0.0, -3.0, 1.0), Vec3::new(4.0, 1.0, 5.0));
        let union = first.union(second);

        assert_eq!(union.min, Vec3::new(-1.0, -3.0, 1.0));
        assert_eq!(union.max, Vec3::new(4.0, 2.0, 5.0));
    }

    #[test]
    fn intersects_ray_from_outside_and_inside() {
        let bounds = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));

        assert!(bounds.intersects_ray(
            Vec3::new(0.0, 0.0, 3.0),
            Vec3::new(f32::INFINITY, f32::INFINITY, -1.0),
            0.0,
            f32::INFINITY,
        ));
        assert!(bounds.intersects_ray(
            Vec3::ZERO,
            Vec3::new(1.0, f32::INFINITY, f32::INFINITY),
            0.0,
            f32::INFINITY,
        ));
    }

    #[test]
    fn rejects_parallel_ray_outside_slab() {
        let bounds = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));

        assert!(!bounds.intersects_ray(
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(f32::INFINITY, f32::INFINITY, -1.0),
            0.0,
            f32::INFINITY,
        ));
    }
}
