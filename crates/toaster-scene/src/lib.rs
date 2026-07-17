pub mod animation;
pub mod loader;
pub mod material;
pub mod object;
pub mod scene;
pub mod transform;

pub use animation::{
    Animation, AnimationTarget, AnimationTrack, Interpolation, RotationKeyframe,
    TranslationKeyframe,
};
pub use loader::load_scene;
pub use material::Material;
pub use object::{Sphere, Triangle};
pub use scene::{Background, CameraSettings, RenderSettings, Scene};
