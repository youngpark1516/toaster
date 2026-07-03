pub mod integrator;
pub mod intersect;

pub use integrator::{ray_color, render};
pub use intersect::{intersect_scene, intersect_sphere, intersect_triangle, HitRecord};
