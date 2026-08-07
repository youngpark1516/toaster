//! Validated renderer-neutral scenes, assets, animation, textures, and
//! environment lighting.

/// Animation tracks, validation, interpolation, and scene evaluation.
pub mod animation;
/// Equirectangular environment maps and importance sampling.
pub mod environment;
/// Renderer-neutral frame evaluation contracts and animation adapter.
pub mod evaluation;
/// JSON scene deserialization, asset resolution, and validation.
pub mod loader;
/// Runtime material representations.
pub mod material;
/// Runtime sphere and triangle geometry.
pub mod object;
/// Backend-neutral physics declarations and render-object bindings.
pub mod physics;
/// Top-level scene, camera, render, and background types.
pub mod scene;
/// Renderer-neutral RGBA8 texture storage and sampling.
pub mod texture;
/// Reserved home for reusable object-transform operations.
pub mod transform;

pub use animation::{
    Animation, AnimationTarget, AnimationTrack, Interpolation, RotationKeyframe,
    TranslationKeyframe,
};
pub use environment::{EnvironmentMap, EnvironmentSample};
pub use evaluation::{
    animation_changes, AnimationEvaluator, EvaluatedScene, EvaluationRequest, SceneChanges,
    SceneEvaluator,
};
pub use loader::load_scene;
pub use material::Material;
pub use object::{Sphere, Triangle, TriangleAttributes};
pub use physics::{
    ColliderShape, ObjectBinding, PhysicsSettings, PhysicsType, RigidBodyDeclaration, RigidBodyKind,
};
pub use scene::{Background, CameraSettings, RenderSettings, Scene};
pub use texture::Texture;
pub use transform::{apply_rigid_transform, RigidTransform};
