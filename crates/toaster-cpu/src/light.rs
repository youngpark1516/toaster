//! Emissive-triangle collection and uniform area sampling.

use glam::Vec3;
use rand::Rng;
use toaster_scene::{Material, Scene, Triangle};

#[derive(Clone, Debug)]
/// Cached geometric and radiometric data for one emissive triangle.
struct AreaLight {
    /// Source triangle.
    triangle: Triangle,
    /// Unit geometric normal.
    normal: Vec3,
    /// Linear emitted radiance.
    emission: Vec3,
    /// Surface area used for weighted light selection.
    area: f32,
}

#[derive(Clone, Copy, Debug)]
/// A point sampled from the combined emissive-triangle area distribution.
pub struct LightSample {
    /// World-space point on the selected light.
    pub position: Vec3,
    /// Selected light's geometric normal.
    pub normal: Vec3,
    /// Linear emitted radiance.
    pub emission: Vec3,
    /// Area of the selected triangle.
    pub area: f32,
    /// Probability density per unit area over all emissive triangles.
    pub pdf_area: f32,
}

#[derive(Clone, Debug, Default)]
/// Cached emissive triangles and their combined surface area.
pub struct AreaLights {
    lights: Vec<AreaLight>,
    total_area: f32,
}

impl AreaLights {
    /// Collects positive-area triangles with positive-strength emissive materials.
    pub fn collect(scene: &Scene) -> Self {
        let mut lights = Vec::new();
        let mut total_area = 0.0;

        for triangle in &scene.triangles {
            let Material::Emissive { color, strength } = scene.materials[triangle.material_index]
            else {
                continue;
            };
            let edge1 = triangle.vertices[1] - triangle.vertices[0];
            let edge2 = triangle.vertices[2] - triangle.vertices[0];
            let cross = edge1.cross(edge2);
            let area = 0.5 * cross.length();
            if area <= 0.0 || strength <= 0.0 {
                continue;
            }
            total_area += area;
            lights.push(AreaLight {
                triangle: triangle.clone(),
                normal: cross.normalize(),
                emission: color * strength,
                area,
            });
        }

        Self { lights, total_area }
    }

    /// Returns the number of collected emissive triangles.
    pub fn len(&self) -> usize {
        self.lights.len()
    }

    /// Reports whether no sampleable emissive triangles exist.
    pub fn is_empty(&self) -> bool {
        self.lights.is_empty()
    }

    /// Selects a triangle proportional to area and samples a uniform point on it.
    pub fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Option<LightSample> {
        if self.lights.is_empty() || self.total_area <= 0.0 {
            return None;
        }

        let target = rng.random::<f32>() * self.total_area;
        let mut accumulated_area = 0.0;
        let mut selected = self.lights.last()?;
        for light in &self.lights {
            accumulated_area += light.area;
            if target <= accumulated_area {
                selected = light;
                break;
            }
        }

        let sqrt_u = rng.random::<f32>().sqrt();
        let v = rng.random::<f32>();
        let weights = [1.0 - sqrt_u, sqrt_u * (1.0 - v), sqrt_u * v];
        let position = selected.triangle.vertices[0] * weights[0]
            + selected.triangle.vertices[1] * weights[1]
            + selected.triangle.vertices[2] * weights[2];

        Some(LightSample {
            position,
            normal: selected.normal,
            emission: selected.emission,
            area: selected.area,
            pdf_area: self.total_area.recip(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};
    use toaster_scene::{Background, CameraSettings, RenderSettings};

    fn light_scene() -> Scene {
        Scene {
            camera: CameraSettings {
                position: Vec3::Z,
                look_at: Vec3::ZERO,
                up: Vec3::Y,
                fov_degrees: 45.0,
            },
            render: RenderSettings {
                width: 1,
                height: 1,
                samples: 1,
                max_bounces: 1,
                background: Background::Black,
            },
            materials: vec![
                Material::Emissive {
                    color: Vec3::new(1.0, 0.5, 0.25),
                    strength: 4.0,
                },
                Material::Diffuse { albedo: Vec3::ONE },
            ],
            spheres: Vec::new(),
            triangles: vec![
                Triangle {
                    vertices: [Vec3::ZERO, Vec3::X * 2.0, Vec3::Y],
                    material_index: 0,
                    group: None,
                },
                Triangle {
                    vertices: [Vec3::Z, Vec3::X + Vec3::Z, Vec3::Y + Vec3::Z],
                    material_index: 1,
                    group: None,
                },
            ],
            triangle_attributes: vec![toaster_scene::TriangleAttributes::default(); 2],
            textures: Vec::new(),
            environment: None,
            animation: Default::default(),
            physics: None,
            rigid_bodies: Vec::new(),
            triggers: Vec::new(),
            spawn_points: Vec::new(),
            event_reactions: Vec::new(),
        }
    }

    #[test]
    fn collects_only_emissive_triangles() {
        let lights = AreaLights::collect(&light_scene());
        assert_eq!(lights.len(), 1);
        assert!(!lights.is_empty());
    }

    #[test]
    fn samples_uniform_triangle_with_area_pdf() {
        let lights = AreaLights::collect(&light_scene());
        let mut rng = StdRng::seed_from_u64(12);
        for _ in 0..100 {
            let sample = lights.sample(&mut rng).unwrap();
            assert!(sample.position.x >= 0.0 && sample.position.y >= 0.0);
            assert!(sample.position.x / 2.0 + sample.position.y <= 1.0 + 1e-6);
            assert!((sample.area - 1.0).abs() < 1e-6);
            assert!((sample.pdf_area - 1.0).abs() < 1e-6);
            assert_eq!(sample.emission, Vec3::new(4.0, 2.0, 1.0));
        }
    }
}
