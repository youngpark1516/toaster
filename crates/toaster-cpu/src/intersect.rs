use toaster_core::ray::Ray;
use toaster_scene::Sphere;

#[derive(Clone, Copy, Debug)]
pub struct HitRecord {
    pub distance: f32,
    pub point: glam::Vec3,
    pub normal: glam::Vec3,
    pub front_face: bool,
    pub material_index: usize,
}

pub fn intersect_sphere(
    ray: &Ray,
    sphere: &Sphere,
    min_distance: f32,
    max_distance: f32,
) -> Option<HitRecord> {
    let offset = ray.origin - sphere.center;
    let a = ray.direction.length_squared();
    let half_b = offset.dot(ray.direction);
    let c = offset.length_squared() - sphere.radius * sphere.radius;
    let discriminant = half_b * half_b - a * c;
    if discriminant < 0.0 {
        return None;
    }

    let sqrt_discriminant = discriminant.sqrt();
    let mut distance = (-half_b - sqrt_discriminant) / a;
    if distance < min_distance || distance > max_distance {
        distance = (-half_b + sqrt_discriminant) / a;
        if distance < min_distance || distance > max_distance {
            return None;
        }
    }

    let point = ray.at(distance);
    let outward_normal = (point - sphere.center) / sphere.radius;
    let front_face = ray.direction.dot(outward_normal) < 0.0;
    let normal = if front_face {
        outward_normal
    } else {
        -outward_normal
    };
    Some(HitRecord {
        distance,
        point,
        normal,
        front_face,
        material_index: sphere.material_index,
    })
}

pub fn intersect_scene(ray: &Ray, spheres: &[Sphere], min_distance: f32) -> Option<HitRecord> {
    let mut closest = f32::INFINITY;
    let mut hit = None;
    for sphere in spheres {
        if let Some(candidate) = intersect_sphere(ray, sphere, min_distance, closest) {
            closest = candidate.distance;
            hit = Some(candidate);
        }
    }
    hit
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn sphere(center: Vec3, radius: f32, material_index: usize) -> Sphere {
        Sphere {
            center,
            radius,
            material_index,
        }
    }

    #[test]
    fn misses_sphere() {
        let ray = Ray::new(Vec3::ZERO, Vec3::Y);
        assert!(intersect_sphere(&ray, &sphere(-Vec3::Z, 0.5, 0), 0.001, f32::INFINITY).is_none());
    }

    #[test]
    fn hits_nearest_surface_and_preserves_material() {
        let ray = Ray::new(Vec3::ZERO, -Vec3::Z);
        let hit = intersect_sphere(
            &ray,
            &sphere(Vec3::new(0.0, 0.0, -2.0), 0.5, 3),
            0.001,
            f32::INFINITY,
        )
        .unwrap();
        assert!((hit.distance - 1.5).abs() < 1e-6);
        assert_eq!(hit.normal, Vec3::Z);
        assert!(hit.front_face);
        assert_eq!(hit.material_index, 3);
    }

    #[test]
    fn inside_ray_uses_far_root_and_orients_normal() {
        let ray = Ray::new(Vec3::ZERO, Vec3::X);
        let hit =
            intersect_sphere(&ray, &sphere(Vec3::ZERO, 1.0, 0), 0.001, f32::INFINITY).unwrap();
        assert!((hit.distance - 1.0).abs() < 1e-6);
        assert_eq!(hit.normal, -Vec3::X);
        assert!(!hit.front_face);
    }

    #[test]
    fn scene_returns_nearest_hit() {
        let ray = Ray::new(Vec3::ZERO, -Vec3::Z);
        let spheres = [
            sphere(Vec3::new(0.0, 0.0, -4.0), 1.0, 0),
            sphere(Vec3::new(0.0, 0.0, -2.0), 0.5, 1),
        ];
        assert_eq!(
            intersect_scene(&ray, &spheres, 0.001)
                .unwrap()
                .material_index,
            1
        );
    }
}
