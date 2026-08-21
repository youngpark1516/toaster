//! Emissive-geometry collection and power-weighted light sampling.

use glam::Vec3;
use rand::Rng;
use toaster_core::light::emissive_light_weight;
use toaster_scene::{Material, Scene, Triangle};

#[derive(Clone, Debug)]
enum LightGeometry {
    Triangle(Triangle),
    Sphere { center: Vec3, radius: f32 },
}

#[derive(Clone, Debug)]
/// Cached geometric and radiometric data for one emissive primitive.
struct AreaLight {
    /// Source primitive used for uniform surface sampling.
    geometry: LightGeometry,
    /// Unit triangle normal; sphere normals come from sampled directions.
    triangle_normal: Vec3,
    /// Linear emitted radiance.
    emission: Vec3,
    /// Surface area used for conditional surface density.
    area: f32,
    /// Approximate emitted power used for discrete selection.
    weight: f32,
    /// Inclusive upper CDF bound in unnormalized weight units.
    cumulative_weight: f32,
}

#[derive(Clone, Copy, Debug)]
/// A point sampled from the power-weighted emissive-geometry distribution.
pub struct LightSample {
    /// World-space point on the selected light.
    pub position: Vec3,
    /// Selected light's geometric normal.
    pub normal: Vec3,
    /// Linear emitted radiance.
    pub emission: Vec3,
    /// Area of the selected primitive.
    pub area: f32,
    /// Discrete probability of selecting this emissive primitive.
    pub selection_pdf: f32,
    /// Mixture probability density per unit area over emissive geometry.
    pub pdf_area: f32,
}

#[derive(Clone, Debug, Default)]
/// Cached emissive geometry and its combined approximate emitted power.
pub struct AreaLights {
    lights: Vec<AreaLight>,
    total_weight: f32,
}

impl AreaLights {
    /// Collects positive-power emissive triangles and spheres.
    pub fn collect(scene: &Scene) -> Self {
        let mut lights = Vec::new();
        let mut total_weight = 0.0;

        for triangle in &scene.triangles {
            let Material::Emissive { color, strength } = scene.materials[triangle.material_index]
            else {
                continue;
            };
            let edge1 = triangle.vertices[1] - triangle.vertices[0];
            let edge2 = triangle.vertices[2] - triangle.vertices[0];
            let cross = edge1.cross(edge2);
            let area = 0.5 * cross.length();
            let weight = emissive_light_weight(area, color, strength);
            if weight <= 0.0 {
                continue;
            }
            total_weight += weight;
            lights.push(AreaLight {
                geometry: LightGeometry::Triangle(triangle.clone()),
                triangle_normal: cross.normalize(),
                emission: color * strength,
                area,
                weight,
                cumulative_weight: total_weight,
            });
        }

        for sphere in &scene.spheres {
            let Material::Emissive { color, strength } = scene.materials[sphere.material_index]
            else {
                continue;
            };
            let area = 4.0 * std::f32::consts::PI * sphere.radius * sphere.radius;
            let weight = emissive_light_weight(area, color, strength);
            if weight <= 0.0 {
                continue;
            }
            total_weight += weight;
            lights.push(AreaLight {
                geometry: LightGeometry::Sphere {
                    center: sphere.center,
                    radius: sphere.radius,
                },
                triangle_normal: Vec3::ZERO,
                emission: color * strength,
                area,
                weight,
                cumulative_weight: total_weight,
            });
        }

        Self {
            lights,
            total_weight,
        }
    }

    /// Returns the number of collected emissive primitives.
    pub fn len(&self) -> usize {
        self.lights.len()
    }

    /// Reports whether no sampleable emissive triangles exist.
    pub fn is_empty(&self) -> bool {
        self.lights.is_empty()
    }

    /// Selects a primitive proportional to emitted power and samples its surface uniformly.
    pub fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Option<LightSample> {
        if self.lights.is_empty() || self.total_weight <= 0.0 {
            return None;
        }

        let target = rng.random::<f32>() * self.total_weight;
        let mut selected = self.lights.last()?;
        for light in &self.lights {
            if target < light.cumulative_weight {
                selected = light;
                break;
            }
        }

        let (position, normal) = match &selected.geometry {
            LightGeometry::Triangle(triangle) => {
                let sqrt_u = rng.random::<f32>().sqrt();
                let v = rng.random::<f32>();
                let weights = [1.0 - sqrt_u, sqrt_u * (1.0 - v), sqrt_u * v];
                (
                    triangle.vertices[0] * weights[0]
                        + triangle.vertices[1] * weights[1]
                        + triangle.vertices[2] * weights[2],
                    selected.triangle_normal,
                )
            }
            LightGeometry::Sphere { center, radius } => {
                let z = rng.random_range(-1.0..1.0);
                let azimuth = rng.random_range(0.0..std::f32::consts::TAU);
                let radial = (1.0_f32 - z * z).max(0.0).sqrt();
                let normal = Vec3::new(radial * azimuth.cos(), radial * azimuth.sin(), z);
                (*center + *radius * normal, normal)
            }
        };
        let selection_pdf = selected.weight / self.total_weight;

        Some(LightSample {
            position,
            normal,
            emission: selected.emission,
            area: selected.area,
            selection_pdf,
            pdf_area: selection_pdf / selected.area,
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
            display: Default::default(),
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
            assert!((sample.selection_pdf - 1.0).abs() < 1e-6);
            assert!((sample.pdf_area - 1.0).abs() < 1e-6);
            assert_eq!(sample.emission, Vec3::new(4.0, 2.0, 1.0));
        }
    }

    #[test]
    fn small_powerful_sphere_dominates_large_dim_triangle_distribution() {
        let mut scene = light_scene();
        scene.materials[0] = Material::Emissive {
            color: Vec3::ONE,
            strength: 0.1,
        };
        scene.materials.push(Material::Emissive {
            color: Vec3::ONE,
            strength: 20.0,
        });
        scene.triangles[0].vertices = [Vec3::ZERO, Vec3::X * 4.0, Vec3::Y * 4.0];
        scene.spheres.push(toaster_scene::Sphere {
            center: Vec3::new(0.0, 2.0, 0.0),
            radius: 0.2,
            material_index: 2,
            group: None,
        });

        let lights = AreaLights::collect(&scene);
        assert_eq!(lights.len(), 2);
        let dim = &lights.lights[0];
        let bright = &lights.lights[1];
        assert!((dim.area - 8.0).abs() < 1.0e-6);
        assert!((dim.weight - 0.8).abs() < 1.0e-6);
        assert!((bright.weight - bright.area * 20.0).abs() < 1.0e-6);
        assert!(bright.weight > dim.weight * 10.0);
        assert!((dim.cumulative_weight - dim.weight).abs() < 1.0e-6);
        assert!((bright.cumulative_weight - lights.total_weight).abs() < 1.0e-6);

        let expected_bright_pdf = bright.weight / lights.total_weight;
        let mut rng = StdRng::seed_from_u64(29);
        let mut bright_samples = 0_u32;
        for _ in 0..20_000 {
            let sample = lights.sample(&mut rng).unwrap();
            if (sample.area - bright.area).abs() < 1.0e-6 {
                bright_samples += 1;
                assert!((sample.selection_pdf - expected_bright_pdf).abs() < 1.0e-6);
                assert!((sample.pdf_area - expected_bright_pdf / bright.area).abs() < 1.0e-6);
            }
        }
        let observed_bright_pdf = bright_samples as f32 / 20_000.0;
        assert!((observed_bright_pdf - expected_bright_pdf).abs() < 0.01);
    }
}
