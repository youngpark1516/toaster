//! Deterministic, readable CPU reference path tracer for validated Toaster scenes.

/// Recursive path integration, material scattering, and direct-light sampling.
pub mod integrator;
/// Sphere/triangle tests and BVH-accelerated scene intersection.
pub mod intersect;
/// Emissive-geometry collection and power-weighted sampling.
pub mod light;

pub use integrator::{ray_color, render};
pub use intersect::{intersect_scene, intersect_sphere, intersect_triangle, HitRecord};
pub use light::{AreaLights, LightSample};
