//! Brute-force CPU ray intersections.

use toaster_core::ray::Ray;
use toaster_scene::{Scene, Sphere, Triangle, TriangleAttributes};

#[derive(Clone, Copy, Debug)]
/// Nearest surface information returned by an intersection query.
pub struct HitRecord {
    /// Parametric distance along the ray.
    pub distance: f32,
    /// World-space hit point.
    pub point: glam::Vec3,
    /// Shading normal oriented against the incoming ray.
    pub normal: glam::Vec3,
    /// Whether the ray hit the geometric front face.
    pub front_face: bool,
    /// Index into the scene material array.
    pub material_index: usize,
    /// Interpolated texture coordinate, or zero when unavailable.
    pub tex_coord: glam::Vec2,
}

/// Finds the nearest sphere hit inside the inclusive distance interval.
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
        tex_coord: glam::Vec2::ZERO,
    })
}

/// Finds a double-sided triangle hit without optional vertex attributes.
pub fn intersect_triangle(
    ray: &Ray,
    triangle: &Triangle,
    min_distance: f32,
    max_distance: f32,
) -> Option<HitRecord> {
    intersect_triangle_with_attributes(
        ray,
        triangle,
        TriangleAttributes::default(),
        min_distance,
        max_distance,
    )
}

/// Applies Möller–Trumbore intersection and interpolates optional attributes.
fn intersect_triangle_with_attributes(
    ray: &Ray,
    triangle: &Triangle,
    attributes: TriangleAttributes,
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

    let geometric_normal = edge1.cross(edge2).normalize();
    let front_face = ray.direction.dot(geometric_normal) < 0.0;
    let weight0 = 1.0 - u - v;
    let mut outward_normal = attributes
        .normals
        .map(|normals| (weight0 * normals[0] + u * normals[1] + v * normals[2]).normalize())
        .unwrap_or(geometric_normal);
    if outward_normal.dot(geometric_normal) < 0.0 {
        outward_normal = -outward_normal;
    }
    let tex_coord = attributes
        .tex_coords
        .map(|tex_coords| weight0 * tex_coords[0] + u * tex_coords[1] + v * tex_coords[2])
        .unwrap_or(glam::Vec2::ZERO);
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
        tex_coord,
    })
}

/// Brute-force scans all primitives and returns the closest valid hit.
pub fn intersect_scene(ray: &Ray, scene: &Scene, min_distance: f32) -> Option<HitRecord> {
    let mut closest = f32::INFINITY;
    let mut hit = None;
    for sphere in &scene.spheres {
        if let Some(candidate) = intersect_sphere(ray, sphere, min_distance, closest) {
            closest = candidate.distance;
            hit = Some(candidate);
        }
    }
    for (triangle_index, triangle) in scene.triangles.iter().enumerate() {
        let attributes = scene
            .triangle_attributes
            .get(triangle_index)
            .copied()
            .unwrap_or_default();
        if let Some(candidate) =
            intersect_triangle_with_attributes(ray, triangle, attributes, min_distance, closest)
        {
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
            triangle_attributes: Vec::new(),
            textures: Vec::new(),
            environment: None,
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

    #[test]
    fn triangle_interpolates_smooth_normal_and_texture_coordinates() {
        let triangle = Triangle {
            vertices: [
                Vec3::new(-1.0, -1.0, -2.0),
                Vec3::new(1.0, -1.0, -2.0),
                Vec3::new(0.0, 1.0, -2.0),
            ],
            material_index: 0,
            group: None,
        };
        let attributes = TriangleAttributes {
            normals: Some([Vec3::Z, Vec3::Z, Vec3::Z]),
            tex_coords: Some([
                glam::Vec2::new(0.0, 0.0),
                glam::Vec2::new(1.0, 0.0),
                glam::Vec2::new(0.5, 1.0),
            ]),
        };

        let hit = intersect_triangle_with_attributes(
            &Ray::new(Vec3::ZERO, -Vec3::Z),
            &triangle,
            attributes,
            0.001,
            f32::INFINITY,
        )
        .unwrap();

        assert!(hit.normal.abs_diff_eq(Vec3::Z, 1e-6));
        assert!(hit.tex_coord.abs_diff_eq(glam::Vec2::new(0.5, 0.5), 1e-6));
    }
}
