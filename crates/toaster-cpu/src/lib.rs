pub mod integrator;
pub mod intersect;
pub mod light;

pub use integrator::{ray_color, render};
pub use intersect::{intersect_scene, intersect_sphere, intersect_triangle, HitRecord};
pub use light::{AreaLights, LightSample};
