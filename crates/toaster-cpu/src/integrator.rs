use crate::intersect::intersect_scene;
use glam::Vec3;
use rand::{rngs::StdRng, Rng, SeedableRng};
use toaster_core::{camera::Camera, image_buffer::ImageBuffer, ray::Ray};
use toaster_scene::Scene;

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

    if let Some(hit) = intersect_scene(ray, &scene.spheres, 0.001) {
        let mut scatter_direction = hit.normal + random_unit_vector(rng);
        if scatter_direction.length_squared() < 1e-8 {
            scatter_direction = hit.normal;
        }

        let scattered = Ray::new(hit.point, scatter_direction.normalize());
        return scene.materials[hit.material_index].albedo
            * ray_color(&scattered, scene, rng, remaining_depth - 1);
    }

    let direction = ray.direction.normalize();
    let blend = 0.5 * (direction.y + 1.0);
    Vec3::ONE.lerp(Vec3::new(0.35, 0.65, 1.0), blend)
}

fn random_unit_vector<R: Rng + ?Sized>(rng: &mut R) -> Vec3 {
    loop {
        let vector = Vec3::new(
            rng.random_range(-1.0..1.0),
            rng.random_range(-1.0..1.0),
            rng.random_range(-1.0..1.0),
        );
        let length_squared = vector.length_squared();
        if (1e-8..=1.0).contains(&length_squared) {
            return vector / length_squared.sqrt();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use toaster_scene::{CameraSettings, Material, RenderSettings, Sphere};

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
            },
            materials: vec![Material { albedo }],
            spheres: vec![Sphere {
                center: Vec3::new(0.0, 0.0, -2.0),
                radius: 0.7,
                material_index: 0,
            }],
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
    fn renders_foreground_and_background() {
        let scene = test_scene(Vec3::new(1.0, 0.0, 0.0));
        let image = render(&scene);
        assert_eq!((image.width(), image.height()), (5, 5));
        assert_ne!(image.pixel(2, 2), image.pixel(0, 0));
    }
}
