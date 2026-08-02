pub mod animation;
pub mod environment;
pub mod loader;
pub mod material;
pub mod object;
pub mod scene;
pub mod texture;
pub mod transform;

pub use animation::{
    Animation, AnimationTarget, AnimationTrack, Interpolation, RotationKeyframe,
    TranslationKeyframe,
};
pub use environment::{EnvironmentMap, EnvironmentSample};
pub use loader::load_scene;
pub use material::Material;
pub use object::{Sphere, Triangle, TriangleAttributes};
pub use scene::{Background, CameraSettings, RenderSettings, Scene};
pub use texture::Texture;
