//! BVH-accelerated CPU ray intersections.

use glam::Vec3;
use toaster_bvh::{Aabb, FlatBvh, PrimitiveInfo, PrimitiveRef};
use toaster_core::ray::Ray;
use toaster_scene::{Scene, Sphere, Triangle, TriangleAttributes};

const TRIANGLE_BOUNDS_PADDING: f32 = 1e-5;

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

#[derive(Clone, Debug, Default)]
/// Flattened acceleration structure built for one evaluated scene.
pub struct SceneBvh {
    flat: FlatBvh,
}

impl SceneBvh {
    /// Builds a hierarchy over all spheres and triangles in `scene`.
    pub fn build(scene: &Scene) -> Self {
        let mut primitives = primitive_info_for_scene(scene);
        Self {
            flat: FlatBvh::build(&mut primitives),
        }
    }

    /// Returns whether the source scene contained no primitives.
    pub fn is_empty(&self) -> bool {
        self.flat.is_empty()
    }
}

/// Traverses the scene hierarchy and returns the closest valid hit.
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
                    PrimitiveRef::Triangle(index) => {
                        let index = index as usize;
                        let attributes = scene
                            .triangle_attributes
                            .get(index)
                            .copied()
                            .unwrap_or_default();
                        intersect_triangle_with_attributes(
                            ray,
                            &scene.triangles[index],
                            attributes,
                            min_distance,
                            closest,
                        )
                    }
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

/// Brute-force reference used to verify accelerated traversal.
pub fn intersect_scene_linear(ray: &Ray, scene: &Scene, min_distance: f32) -> Option<HitRecord> {
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
            display: Default::default(),
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
            physics: None,
            rigid_bodies: Vec::new(),
            triggers: Vec::new(),
            spawn_points: Vec::new(),
            event_reactions: Vec::new(),
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
            display: Default::default(),
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
            triangle_attributes: vec![TriangleAttributes::default()],
            textures: Vec::new(),
            environment: None,
            animation: Default::default(),
            physics: None,
            rigid_bodies: Vec::new(),
            triggers: Vec::new(),
            spawn_points: Vec::new(),
            event_reactions: Vec::new(),
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

    #[test]
    fn cpu_bvh_accepts_neutrally_evaluated_physics_geometry() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/011_physics_rigid_bodies.json");
        let source = toaster_scene::load_scene(path).unwrap();
        let sphere_binding = source
            .rigid_bodies
            .iter()
            .find(|body| body.group.as_deref() == Some("ball_0"))
            .unwrap()
            .binding
            .clone();
        let box_binding = source
            .rigid_bodies
            .iter()
            .find(|body| body.group.as_deref() == Some("crate_0"))
            .unwrap()
            .binding
            .clone();
        let mut evaluated = source.clone();
        toaster_scene::apply_rigid_transform(
            &source,
            &mut evaluated,
            &sphere_binding,
            toaster_scene::RigidTransform {
                translation: Vec3::new(0.0, 1.0, -2.0),
                rotation: glam::Quat::IDENTITY,
            },
        )
        .unwrap();
        toaster_scene::apply_rigid_transform(
            &source,
            &mut evaluated,
            &box_binding,
            toaster_scene::RigidTransform {
                translation: Vec3::new(1.5, 1.0, -3.0),
                rotation: glam::Quat::from_rotation_y(0.6),
            },
        )
        .unwrap();
        let bvh = SceneBvh::build(&evaluated);

        for target in [Vec3::new(0.0, 1.0, -2.0), Vec3::new(1.5, 1.0, -3.0)] {
            let ray = Ray::new(
                evaluated.camera.position,
                (target - evaluated.camera.position).normalize(),
            );
            let linear = intersect_scene_linear(&ray, &evaluated, 0.001).unwrap();
            let accelerated = intersect_scene(&ray, &evaluated, &bvh, 0.001).unwrap();
            assert!((linear.distance - accelerated.distance).abs() < 1.0e-5);
            assert_eq!(linear.material_index, accelerated.material_index);
        }
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
