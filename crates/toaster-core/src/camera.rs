//! Pinhole camera construction and primary-ray generation.

use crate::ray::Ray;
use glam::Vec3;

#[derive(Clone, Copy, Debug)]
/// A validated pinhole camera represented by its viewport basis.
pub struct Camera {
    origin: Vec3,
    lower_left: Vec3,
    horizontal: Vec3,
    vertical: Vec3,
}

impl Camera {
    /// Builds a camera looking from `position` toward `look_at`.
    ///
    /// Returns an error when the field of view or aspect ratio is invalid, the
    /// view direction has no length, or `up` is parallel to that direction.
    ///
    /// ```
    /// use glam::Vec3;
    /// use toaster_core::camera::Camera;
    ///
    /// let camera = Camera::new(Vec3::new(0.0, 0.0, 2.0), Vec3::ZERO, Vec3::Y, 45.0, 16.0 / 9.0)?;
    /// let center_ray = camera.ray(0.5, 0.5);
    /// assert!(center_ray.direction.z < 0.0);
    /// # Ok::<(), &'static str>(())
    /// ```
    pub fn new(
        position: Vec3,
        look_at: Vec3,
        up: Vec3,
        fov_degrees: f32,
        aspect_ratio: f32,
    ) -> Result<Self, &'static str> {
        if !(0.0..180.0).contains(&fov_degrees) || !aspect_ratio.is_finite() || aspect_ratio <= 0.0
        {
            return Err("camera FOV and aspect ratio must be positive and finite");
        }

        let view = position - look_at;
        if !view.is_finite() || view.length_squared() <= f32::EPSILON {
            return Err("camera position and look_at must differ");
        }

        let backward = view.normalize();
        let right = up.cross(backward);
        if !right.is_finite() || right.length_squared() <= f32::EPSILON {
            return Err("camera up vector must not be parallel to the view direction");
        }

        let right = right.normalize();
        let true_up = backward.cross(right);
        let viewport_height = 2.0 * (0.5 * fov_degrees.to_radians()).tan();
        let viewport_width = aspect_ratio * viewport_height;
        let horizontal = viewport_width * right;
        let vertical = viewport_height * true_up;
        let lower_left = position - horizontal * 0.5 - vertical * 0.5 - backward;

        Ok(Self {
            origin: position,
            lower_left,
            horizontal,
            vertical,
        })
    }

    /// Returns the normalized primary ray through viewport coordinates `(u, v)`.
    pub fn ray(&self, u: f32, v: f32) -> Ray {
        Ray::new(
            self.origin,
            (self.lower_left + u * self.horizontal + v * self.vertical - self.origin).normalize(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_ray_points_at_target() {
        let camera = Camera::new(Vec3::ZERO, -Vec3::Z, Vec3::Y, 60.0, 1.0).unwrap();
        assert!(camera.ray(0.5, 0.5).direction.abs_diff_eq(-Vec3::Z, 1e-6));
    }

    #[test]
    fn rejects_non_finite_aspect_ratio() {
        assert!(Camera::new(Vec3::ZERO, -Vec3::Z, Vec3::Y, 60.0, f32::NAN).is_err());
        assert!(Camera::new(Vec3::ZERO, -Vec3::Z, Vec3::Y, 60.0, f32::INFINITY).is_err());
    }
}
