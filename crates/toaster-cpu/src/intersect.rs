use glam::Vec3;
use toaster_bvh::{Aabb, FlatBvh, PrimitiveInfo, PrimitiveRef};
use toaster_core::ray::Ray;
use toaster_scene::{Scene, Sphere, Triangle};

const TRIANGLE_BOUNDS_PADDING: f32 = 1e-5;

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

#[derive(Clone, Debug, Default)]
pub struct SceneBvh {
    flat: FlatBvh,
}

impl SceneBvh {
    pub fn build(scene: &Scene) -> Self {
        let mut primitives = primitive_info_for_scene(scene);
        Self {
            flat: FlatBvh::build(&mut primitives),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.flat.is_empty()
    }
}

pub fn intersect_scene(
    ray: &Ray,
    scene: &Scene,
    bvh: &SceneBvh,
    min_distance: f32,
) -> Option<HitRecord> {
    if bvh.is_empty() {
        return None;
    }

    let inverse_direction = Vec3::new(
        ray.direction.x.recip(),
        ray.direction.y.recip(),
        ray.direction.z.recip(),
    );
    let mut closest = f32::INFINITY;
    let mut hit = None;
    let mut stack = vec![0_u32];

    while let Some(node_index) = stack.pop() {
        let node = bvh.flat.nodes[node_index as usize];
        if !node
            .bounds
            .intersects_ray(ray.origin, inverse_direction, min_distance, closest)
        {
            continue;
        }

        if node.is_leaf() {
            let first = node.first_primitive() as usize;
            let end = first + node.primitive_count as usize;
            for primitive in &bvh.flat.primitives[first..end] {
                let candidate = match *primitive {
                    PrimitiveRef::Sphere(index) => {
                        intersect_sphere(ray, &scene.spheres[index as usize], min_distance, closest)
                    }
                    PrimitiveRef::Triangle(index) => intersect_triangle(
                        ray,
                        &scene.triangles[index as usize],
                        min_distance,
                        closest,
                    ),
                };
                if let Some(candidate) = candidate {
                    closest = candidate.distance;
                    hit = Some(candidate);
                }
            }
        } else {
            let left_child = node_index + 1;
            let right_child = node.right_child();
            let left = bvh.flat.nodes[left_child as usize];
            let right = bvh.flat.nodes[right_child as usize];
            let left_hit =
                left.bounds
                    .intersects_ray(ray.origin, inverse_direction, min_distance, closest);
            let right_hit =
                right
                    .bounds
                    .intersects_ray(ray.origin, inverse_direction, min_distance, closest);

            match (left_hit, right_hit) {
                (true, true) => {
                    let left_distance = entry_distance(left.bounds, ray.origin, inverse_direction);
                    let right_distance =
                        entry_distance(right.bounds, ray.origin, inverse_direction);
                    if left_distance < right_distance {
                        stack.push(right_child);
                        stack.push(left_child);
                    } else {
                        stack.push(left_child);
                        stack.push(right_child);
                    }
                }
                (true, false) => stack.push(left_child),
                (false, true) => stack.push(right_child),
                (false, false) => {}
            }
        }
    }

    hit
}

pub fn intersect_scene_linear(ray: &Ray, scene: &Scene, min_distance: f32) -> Option<HitRecord> {
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

fn primitive_info_for_scene(scene: &Scene) -> Vec<PrimitiveInfo> {
    let mut primitives = Vec::with_capacity(scene.spheres.len() + scene.triangles.len());

    primitives.extend(scene.spheres.iter().enumerate().map(|(index, sphere)| {
        let radius = Vec3::splat(sphere.radius);
        let bounds = Aabb::new(sphere.center - radius, sphere.center + radius);
        PrimitiveInfo {
            bounds,
            centroid: bounds.centroid(),
            primitive: PrimitiveRef::Sphere(index as u32),
        }
    }));

    primitives.extend(scene.triangles.iter().enumerate().map(|(index, triangle)| {
        let bounds = triangle_bounds(triangle);
        PrimitiveInfo {
            bounds,
            centroid: bounds.centroid(),
            primitive: PrimitiveRef::Triangle(index as u32),
        }
    }));

    primitives
}

fn triangle_bounds(triangle: &Triangle) -> Aabb {
    let [first, second, third] = triangle.vertices;
    Aabb::new(first.min(second).min(third), first.max(second).max(third))
        .expand(TRIANGLE_BOUNDS_PADDING)
}

fn entry_distance(bounds: Aabb, origin: Vec3, inverse_direction: Vec3) -> f32 {
    let mut entry = f32::NEG_INFINITY;

    for axis in 0..3 {
        if inverse_direction[axis].is_infinite() {
            continue;
        }
        let mut near = (bounds.min[axis] - origin[axis]) * inverse_direction[axis];
        let mut far = (bounds.max[axis] - origin[axis]) * inverse_direction[axis];
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        entry = entry.max(near);
    }

    entry
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
        let bvh = SceneBvh::build(&scene);
        assert_eq!(
            intersect_scene(&ray, &scene, &bvh, 0.001)
                .unwrap()
                .material_index,
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
    fn bvh_matches_linear_intersection_for_mixed_scene() {
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
                Material::Diffuse { albedo: Vec3::ONE },
            ],
            spheres: vec![
                sphere(Vec3::new(-1.5, 0.0, -4.0), 0.75, 0),
                sphere(Vec3::new(1.25, 0.0, -3.0), 0.5, 1),
            ],
            triangles: vec![Triangle {
                vertices: [
                    Vec3::new(-1.0, -1.0, -2.0),
                    Vec3::new(1.0, -1.0, -2.0),
                    Vec3::new(0.0, 1.0, -2.0),
                ],
                material_index: 2,
                group: None,
            }],
            animation: Default::default(),
        };
        let bvh = SceneBvh::build(&scene);
        let rays = [
            Ray::new(Vec3::ZERO, -Vec3::Z),
            Ray::new(Vec3::ZERO, Vec3::new(-0.4, 0.0, -1.0).normalize()),
            Ray::new(Vec3::ZERO, Vec3::new(0.45, 0.0, -1.0).normalize()),
            Ray::new(Vec3::new(0.0, 0.0, -3.0), Vec3::Y),
            Ray::new(Vec3::new(5.0, 5.0, 0.0), -Vec3::Z),
        ];

        for ray in rays {
            let linear = intersect_scene_linear(&ray, &scene, 0.001);
            let accelerated = intersect_scene(&ray, &scene, &bvh, 0.001);
            match (linear, accelerated) {
                (Some(linear), Some(accelerated)) => {
                    assert!((linear.distance - accelerated.distance).abs() < 1e-5);
                    assert_eq!(linear.material_index, accelerated.material_index);
                    assert_eq!(linear.front_face, accelerated.front_face);
                    assert!(linear.normal.abs_diff_eq(accelerated.normal, 1e-5));
                }
                (None, None) => {}
                (linear, accelerated) => {
                    panic!("linear hit {linear:?} did not match bvh hit {accelerated:?}");
                }
            }
        }
    }
}
