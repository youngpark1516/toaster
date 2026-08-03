//! Runtime material descriptions shared by the CPU and GPU upload paths.

use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
/// A validated material attached to scene geometry by index.
pub enum Material {
    /// Lambertian reflection with a constant linear base color.
    Diffuse {
        /// Linear RGB reflectance in `[0, 1]`.
        albedo: Vec3,
    },
    /// Lambertian reflection modulated by a base-color texture.
    TexturedDiffuse {
        /// Linear RGB factor multiplied with the sampled texture.
        albedo: Vec3,
        /// Index into [`crate::Scene::textures`].
        texture_index: usize,
    },
    /// Fuzzy specular reflection.
    Metal {
        /// Linear RGB reflection tint.
        albedo: Vec3,
        /// Reflection perturbation in `[0, 1]`.
        roughness: f32,
    },
    /// Ideal reflective/refractive dielectric.
    Dielectric {
        /// Positive index of refraction.
        ior: f32,
    },
    /// Non-scattering surface emission.
    Emissive {
        /// Linear RGB emission color.
        color: Vec3,
        /// Nonnegative emission multiplier.
        strength: f32,
    },
}
