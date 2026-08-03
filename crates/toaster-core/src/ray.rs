//! Geometric rays.

use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
/// A ray with an origin and direction.
pub struct Ray {
    /// World-space starting point.
    pub origin: Vec3,
    /// World-space travel direction, normally normalized by the caller.
    pub direction: Vec3,
}

impl Ray {
    /// Creates a ray without modifying or normalizing its direction.
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self { origin, direction }
    }

    /// Evaluates the parametric ray at `origin + distance * direction`.
    pub fn at(&self, distance: f32) -> Vec3 {
        self.origin + distance * self.direction
    }
}
