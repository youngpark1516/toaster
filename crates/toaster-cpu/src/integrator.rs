use crate::intersect::intersect_scene;
use glam::Vec3;
use rand::{rngs::StdRng, Rng, SeedableRng};
use toaster_core::{camera::Camera, image_buffer::ImageBuffer, ray::Ray};
use toaster_scene::Scene;

const RENDER_SEED: u64 = 0x544f_4153_5445_52;

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
                color += shade(&camera.ray(u, v), scene);
            }
            image.set_pixel(x, y, color / settings.samples as f32);
        }
    }
    image
}

fn shade(ray: &Ray, scene: &Scene) -> Vec3 {
    if let Some(hit) = intersect_scene(ray, &scene.spheres, 0.001) {
        let light_direction = Vec3::new(1.0, 1.0, 0.5).normalize();
        let diffuse = hit.normal.dot(light_direction).max(0.0);
        return scene.materials[hit.material_index].albedo * (0.2 + 0.8 * diffuse);
    }

    let direction = ray.direction.normalize();
    let blend = 0.5 * (direction.y + 1.0);
    Vec3::ONE.lerp(Vec3::new(0.35, 0.65, 1.0), blend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use toaster_scene::{CameraSettings, Material, RenderSettings, Sphere};

    #[test]
    fn renders_foreground_and_background() {
        let scene = Scene {
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
                max_bounces: 1,
            },
            materials: vec![Material {
                albedo: Vec3::new(1.0, 0.0, 0.0),
            }],
            spheres: vec![Sphere {
                center: Vec3::new(0.0, 0.0, -2.0),
                radius: 0.7,
                material_index: 0,
            }],
        };
        let image = render(&scene);
        assert_eq!((image.width(), image.height()), (5, 5));
        assert_ne!(image.pixel(2, 2), image.pixel(0, 0));
    }
}
