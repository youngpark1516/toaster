use toaster_core::ray::Ray;
use toaster_scene::{Scene, Sphere, Triangle};

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

pub fn intersect_triangle(
    ray: &Ray,
    triangle: &Triangle,
    min_distance: f32,
    max_distance: f32,
) -> Option<HitRecord> {
    const EPSILON: f32 = 1e-8;

    let edge1 = triangle.vertices[1] - triangle.vertices[0];
    let edge2 = triangle.vertices[2] - triangle.vertices[0];
    let direction_cross_edge2 = ray.direction.cross(edge2);
    let determinant = edge1.dot(direction_cross_edge2);
    if determinant.abs() < EPSILON {
        return None;
    }

    let inverse_determinant = determinant.recip();
    let origin_offset = ray.origin - triangle.vertices[0];
    let u = origin_offset.dot(direction_cross_edge2) * inverse_determinant;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }

    let origin_cross_edge1 = origin_offset.cross(edge1);
    let v = ray.direction.dot(origin_cross_edge1) * inverse_determinant;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }

    let distance = edge2.dot(origin_cross_edge1) * inverse_determinant;
    if distance < min_distance || distance > max_distance {
        return None;
    }

    let outward_normal = edge1.cross(edge2).normalize();
    let front_face = ray.direction.dot(outward_normal) < 0.0;
    Some(HitRecord {
        distance,
        point: ray.at(distance),
        normal: if front_face {
            outward_normal
        } else {
            -outward_normal
        },
        front_face,
        material_index: triangle.material_index,
    })
}

pub fn intersect_scene(ray: &Ray, scene: &Scene, min_distance: f32) -> Option<HitRecord> {
    let mut closest = f32::INFINITY;
    let mut hit = None;
    for sphere in &scene.spheres {
        if let Some(candidate) = intersect_sphere(ray, sphere, min_distance, closest) {
            closest = candidate.distance;
            hit = Some(candidate);
        }
    }
    for triangle in &scene.triangles {
        if let Some(candidate) = intersect_triangle(ray, triangle, min_distance, closest) {
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
    use toaster_scene::{Background, CameraSettings, Material, RenderSettings};

    fn sphere(center: Vec3, radius: f32, material_index: usize) -> Sphere {
        Sphere {
            center,
            radius,
            material_index,
            group: None,
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
        let scene = Scene {
            camera: CameraSettings {
                position: Vec3::ZERO,
                look_at: -Vec3::Z,
                up: Vec3::Y,
                fov_degrees: 60.0,
            },
            render: RenderSettings {
                width: 1,
                height: 1,
                samples: 1,
                max_bounces: 1,
                background: Background::Sky,
            },
            materials: vec![
                Material::Diffuse { albedo: Vec3::ONE },
                Material::Diffuse { albedo: Vec3::ONE },
            ],
            spheres: vec![
                sphere(Vec3::new(0.0, 0.0, -4.0), 1.0, 0),
                sphere(Vec3::new(0.0, 0.0, -2.0), 0.5, 1),
            ],
            triangles: Vec::new(),
            animation: Default::default(),
        };
        assert_eq!(
            intersect_scene(&ray, &scene, 0.001).unwrap().material_index,
            1
        );
    }

    #[test]
    fn triangle_hit_is_double_sided_and_reports_nearest_distance() {
        let triangle = Triangle {
            vertices: [
                Vec3::new(-1.0, -1.0, -2.0),
                Vec3::new(1.0, -1.0, -2.0),
                Vec3::new(0.0, 1.0, -2.0),
            ],
            material_index: 7,
            group: None,
        };
        let front = intersect_triangle(
            &Ray::new(Vec3::ZERO, -Vec3::Z),
            &triangle,
            0.001,
            f32::INFINITY,
        )
        .unwrap();
        assert!((front.distance - 2.0).abs() < 1e-6);
        assert!(front.front_face);
        assert_eq!(front.material_index, 7);

        let back = intersect_triangle(
            &Ray::new(Vec3::new(0.0, 0.0, -3.0), Vec3::Z),
            &triangle,
            0.001,
            f32::INFINITY,
        )
        .unwrap();
        assert!(!back.front_face);
        assert_eq!(back.normal, -front.normal);
    }

    #[test]
    fn triangle_misses_outside_its_edges() {
        let triangle = Triangle {
            vertices: [Vec3::ZERO, Vec3::X, Vec3::Y],
            material_index: 0,
            group: None,
        };
        assert!(intersect_triangle(
            &Ray::new(Vec3::new(2.0, 2.0, 1.0), -Vec3::Z),
            &triangle,
            0.001,
            f32::INFINITY,
        )
        .is_none());
    }
}
