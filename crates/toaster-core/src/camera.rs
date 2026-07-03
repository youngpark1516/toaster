use crate::ray::Ray;
use glam::Vec3;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    origin: Vec3,
    lower_left: Vec3,
    horizontal: Vec3,
    vertical: Vec3,
}

impl Camera {
    pub fn new(
        position: Vec3,
        look_at: Vec3,
        up: Vec3,
        fov_degrees: f32,
        aspect_ratio: f32,
    ) -> Result<Self, &'static str> {
        if !(0.0..180.0).contains(&fov_degrees) || aspect_ratio <= 0.0 {
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
}
