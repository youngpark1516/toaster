use crate::intersect::{intersect_scene, HitRecord};
use glam::Vec3;
use rand::{rngs::StdRng, Rng, SeedableRng};
use toaster_core::{camera::Camera, image_buffer::ImageBuffer, ray::Ray};
use toaster_scene::{Background, Material, Scene};

const RENDER_SEED: u64 = 0x0054_4f41_5354_4552;

pub fn render(scene: &Scene) -> ImageBuffer {
    let settings = scene.render;
    let camera = Camera::new(
        scene.camera.position,
        scene.camera.look_at,
        scene.camera.up,
        scene.camera.fov_degrees,
        settings.width as f32 / settings.height as f32,
    )
    .expect("scene loader validates camera settings");

    let mut image = ImageBuffer::new(settings.width, settings.height);
    let mut rng = StdRng::seed_from_u64(RENDER_SEED);
    for y in 0..settings.height {
        for x in 0..settings.width {
            let mut color = Vec3::ZERO;
            for _ in 0..settings.samples {
                let u = (x as f32 + rng.random::<f32>()) / settings.width as f32;
                let v = 1.0 - (y as f32 + rng.random::<f32>()) / settings.height as f32;
                color += ray_color(&camera.ray(u, v), scene, &mut rng, settings.max_bounces);
            }
            image.set_pixel(x, y, color / settings.samples as f32);
        }
    }
    image
}

pub fn ray_color<R: Rng + ?Sized>(
    ray: &Ray,
    scene: &Scene,
    rng: &mut R,
    remaining_depth: u32,
) -> Vec3 {
    if remaining_depth == 0 {
        return Vec3::ZERO;
    }

    if let Some(hit) = intersect_scene(ray, scene, 0.001) {
        let material = scene.materials[hit.material_index];
        if let Material::Emissive { color, strength } = material {
            return color * strength;
        }
        if let Some(scatter) = scatter(ray, &hit, material, rng) {
            return scatter.attenuation * ray_color(&scatter.ray, scene, rng, remaining_depth - 1);
        }
        return Vec3::ZERO;
    }

    match scene.render.background {
        Background::Sky => {
            let direction = ray.direction.normalize();
            let blend = 0.5 * (direction.y + 1.0);
            Vec3::ONE.lerp(Vec3::new(0.35, 0.65, 1.0), blend)
        }
        Background::Black => Vec3::ZERO,
    }
}

#[derive(Clone, Copy, Debug)]
struct Scatter {
    ray: Ray,
    attenuation: Vec3,
}

fn scatter<R: Rng + ?Sized>(
    incoming: &Ray,
    hit: &HitRecord,
    material: Material,
    rng: &mut R,
) -> Option<Scatter> {
    match material {
        Material::Diffuse { albedo } => {
            let mut direction = hit.normal + random_unit_vector(rng);
            if direction.length_squared() < 1e-8 {
                direction = hit.normal;
            }
            Some(Scatter {
                ray: Ray::new(hit.point, direction.normalize()),
                attenuation: albedo,
            })
        }
        Material::Metal { albedo, roughness } => {
            let reflected = reflect(incoming.direction.normalize(), hit.normal);
            let direction = reflected + roughness.clamp(0.0, 1.0) * random_in_unit_sphere(rng);
            if direction.dot(hit.normal) <= 0.0 {
                return None;
            }
            Some(Scatter {
                ray: Ray::new(hit.point, direction.normalize()),
                attenuation: albedo,
            })
        }
        Material::Dielectric { ior } => {
            let attenuation = Vec3::ONE;
            let refraction_ratio = if hit.front_face { 1.0 / ior } else { ior };
            let unit_direction = incoming.direction.normalize();
            let cos_theta = (-unit_direction).dot(hit.normal).min(1.0);
            let sin_theta = (1.0 - cos_theta * cos_theta).sqrt();
            let cannot_refract = refraction_ratio * sin_theta > 1.0;
            let direction = if cannot_refract
                || reflectance(cos_theta, refraction_ratio) > rng.random::<f32>()
            {
                reflect(unit_direction, hit.normal)
            } else {
                refract(unit_direction, hit.normal, refraction_ratio)
            };
            Some(Scatter {
                ray: Ray::new(hit.point, direction.normalize()),
                attenuation,
            })
        }
        Material::Emissive { .. } => None,
    }
}

fn reflect(direction: Vec3, normal: Vec3) -> Vec3 {
    direction - 2.0 * direction.dot(normal) * normal
}

fn refract(direction: Vec3, normal: Vec3, ratio: f32) -> Vec3 {
    let cos_theta = (-direction).dot(normal).min(1.0);
    let perpendicular = ratio * (direction + cos_theta * normal);
    let parallel = -(1.0 - perpendicular.length_squared()).abs().sqrt() * normal;
    perpendicular + parallel
}

fn reflectance(cosine: f32, refraction_ratio: f32) -> f32 {
    let r0 = ((1.0 - refraction_ratio) / (1.0 + refraction_ratio)).powi(2);
    r0 + (1.0 - r0) * (1.0 - cosine).powi(5)
}

fn random_unit_vector<R: Rng + ?Sized>(rng: &mut R) -> Vec3 {
    random_in_unit_sphere(rng).normalize()
}

fn random_in_unit_sphere<R: Rng + ?Sized>(rng: &mut R) -> Vec3 {
    loop {
        let vector = Vec3::new(
            rng.random_range(-1.0..1.0),
            rng.random_range(-1.0..1.0),
            rng.random_range(-1.0..1.0),
        );
        let length_squared = vector.length_squared();
        if (1e-8..=1.0).contains(&length_squared) {
            return vector;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use toaster_scene::{Background, CameraSettings, Material, RenderSettings, Sphere};

    fn test_scene(albedo: Vec3) -> Scene {
        Scene {
            camera: CameraSettings {
                position: Vec3::ZERO,
                look_at: -Vec3::Z,
                up: Vec3::Y,
                fov_degrees: 60.0,
            },
            render: RenderSettings {
                width: 5,
                height: 5,
                samples: 1,
                max_bounces: 4,
                background: Background::Sky,
            },
            materials: vec![Material::Diffuse { albedo }],
            spheres: vec![Sphere {
                center: Vec3::new(0.0, 0.0, -2.0),
                radius: 0.7,
                material_index: 0,
            }],
            triangles: Vec::new(),
        }
    }

    fn test_hit(normal: Vec3, front_face: bool) -> HitRecord {
        HitRecord {
            distance: 1.0,
            point: Vec3::ZERO,
            normal,
            front_face,
            material_index: 0,
        }
    }

    #[test]
    fn random_unit_vectors_are_finite_and_normalized() {
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..100 {
            let vector = random_unit_vector(&mut rng);
            assert!(vector.is_finite());
            assert!((vector.length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn zero_depth_is_black() {
        let mut rng = StdRng::seed_from_u64(1);
        let color = ray_color(
            &Ray::new(Vec3::ZERO, Vec3::Y),
            &test_scene(Vec3::ONE),
            &mut rng,
            0,
        );
        assert_eq!(color, Vec3::ZERO);
    }

    #[test]
    fn miss_returns_sky_gradient() {
        let mut rng = StdRng::seed_from_u64(1);
        let color = ray_color(
            &Ray::new(Vec3::ZERO, Vec3::Y),
            &test_scene(Vec3::ONE),
            &mut rng,
            4,
        );
        assert!(color.abs_diff_eq(Vec3::new(0.35, 0.65, 1.0), 1e-6));
    }

    #[test]
    fn diffuse_hit_attenuates_returned_radiance_by_albedo() {
        let ray = Ray::new(Vec3::ZERO, -Vec3::Z);
        let albedo = Vec3::new(0.8, 0.4, 0.2);
        let mut white_rng = StdRng::seed_from_u64(7);
        let mut colored_rng = StdRng::seed_from_u64(7);
        let incoming = ray_color(&ray, &test_scene(Vec3::ONE), &mut white_rng, 2);
        let attenuated = ray_color(&ray, &test_scene(albedo), &mut colored_rng, 2);
        assert!(attenuated.abs_diff_eq(incoming * albedo, 1e-6));
    }

    #[test]
    fn perfect_metal_reflects_and_below_surface_ray_is_absorbed() {
        let mut rng = StdRng::seed_from_u64(3);
        let material = Material::Metal {
            albedo: Vec3::splat(0.8),
            roughness: 0.0,
        };
        let reflected = scatter(
            &Ray::new(Vec3::ZERO, Vec3::new(1.0, -1.0, 0.0)),
            &test_hit(Vec3::Y, true),
            material,
            &mut rng,
        )
        .unwrap();
        assert!(reflected
            .ray
            .direction
            .abs_diff_eq(Vec3::new(1.0, 1.0, 0.0).normalize(), 1e-6));

        let absorbed = scatter(
            &Ray::new(Vec3::ZERO, Vec3::Y),
            &test_hit(Vec3::Y, false),
            material,
            &mut rng,
        );
        assert!(absorbed.is_none());
    }

    #[test]
    fn metal_scatter_clamps_roughness_defensively() {
        let mut clamped_rng = StdRng::seed_from_u64(4);
        let mut oversized_rng = StdRng::seed_from_u64(4);
        let incoming = Ray::new(Vec3::ZERO, -Vec3::Y);
        let hit = test_hit(Vec3::Y, true);
        let clamped = scatter(
            &incoming,
            &hit,
            Material::Metal {
                albedo: Vec3::ONE,
                roughness: 1.0,
            },
            &mut clamped_rng,
        )
        .unwrap();
        let oversized = scatter(
            &incoming,
            &hit,
            Material::Metal {
                albedo: Vec3::ONE,
                roughness: 5.0,
            },
            &mut oversized_rng,
        )
        .unwrap();
        assert!(clamped
            .ray
            .direction
            .abs_diff_eq(oversized.ray.direction, 1e-6));
    }

    #[test]
    fn dielectric_refraction_obeys_snells_law() {
        let direction = Vec3::new(0.5, -3.0_f32.sqrt() * 0.5, 0.0);
        let refracted = refract(direction, Vec3::Y, 1.0 / 1.5);
        assert!((refracted.x - 1.0 / 3.0).abs() < 1e-6);
        assert!((refracted.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn dielectric_uses_total_internal_reflection() {
        let incoming = Ray::new(Vec3::ZERO, Vec3::new(0.9, 0.435_889_9, 0.0));
        let hit = test_hit(-Vec3::Y, false);
        let mut rng = StdRng::seed_from_u64(5);
        let scattered =
            scatter(&incoming, &hit, Material::Dielectric { ior: 1.5 }, &mut rng).unwrap();
        let expected = reflect(incoming.direction.normalize(), hit.normal);
        assert!(scattered.ray.direction.abs_diff_eq(expected, 1e-6));
        assert_eq!(scattered.attenuation, Vec3::ONE);
    }

    #[test]
    fn schlick_reflectance_has_expected_endpoints() {
        assert!((reflectance(0.0, 1.0 / 1.5) - 1.0).abs() < 1e-6);
        assert!((reflectance(1.0, 1.0 / 1.5) - 0.04).abs() < 1e-6);
    }

    #[test]
    fn material_scatter_directions_are_finite_and_normalized() {
        let incoming = Ray::new(Vec3::ZERO, -Vec3::Y);
        let hit = test_hit(Vec3::Y, true);
        let materials = [
            Material::Diffuse { albedo: Vec3::ONE },
            Material::Metal {
                albedo: Vec3::ONE,
                roughness: 0.2,
            },
            Material::Dielectric { ior: 1.5 },
        ];
        for (seed, material) in materials.into_iter().enumerate() {
            let mut rng = StdRng::seed_from_u64(seed as u64);
            let scattered = scatter(&incoming, &hit, material, &mut rng).unwrap();
            assert!(scattered.ray.direction.is_finite());
            assert!((scattered.ray.direction.length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn emissive_hit_returns_emitted_radiance_without_scattering() {
        let mut scene = test_scene(Vec3::ONE);
        scene.materials[0] = Material::Emissive {
            color: Vec3::new(1.0, 0.5, 0.25),
            strength: 4.0,
        };
        let mut rng = StdRng::seed_from_u64(9);
        let color = ray_color(&Ray::new(Vec3::ZERO, -Vec3::Z), &scene, &mut rng, 4);
        assert!(color.abs_diff_eq(Vec3::new(4.0, 2.0, 1.0), 1e-6));
    }

    #[test]
    fn black_background_returns_no_radiance() {
        let mut scene = test_scene(Vec3::ONE);
        scene.render.background = Background::Black;
        let mut rng = StdRng::seed_from_u64(10);
        let color = ray_color(&Ray::new(Vec3::ZERO, Vec3::Y), &scene, &mut rng, 4);
        assert_eq!(color, Vec3::ZERO);
    }

    #[test]
    fn renders_foreground_and_background() {
        let scene = test_scene(Vec3::new(1.0, 0.0, 0.0));
        let image = render(&scene);
        assert_eq!((image.width(), image.height()), (5, 5));
        assert_ne!(image.pixel(2, 2), image.pixel(0, 0));
    }
}
