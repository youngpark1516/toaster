//! Renderer-neutral physics declarations and stable render-object bindings.

use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Simulation domain selected by a scene physics block.
pub enum PhysicsType {
    /// Discrete rigid bodies with collision shapes.
    RigidBody,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Validated top-level physics controls.
pub struct PhysicsSettings {
    /// Whether declarations actively participate in evaluation and ownership.
    pub enabled: bool,
    /// Simulation domain, independent of the concrete backend implementation.
    pub physics_type: PhysicsType,
    /// World-space acceleration in metres per second squared.
    pub gravity: Vec3,
    /// Duration of one nominal fixed tick in seconds.
    pub timestep: f32,
    /// Backend steps executed within each nominal tick.
    pub substeps: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Supported rigid-body ownership modes.
pub enum RigidBodyKind {
    /// Immovable collision geometry.
    Static,
    /// Geometry integrated by the active rigid-body backend.
    Dynamic,
    /// Collision geometry driven by renderer-neutral animation targets.
    Kinematic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Renderer-neutral collider geometry attached to a rigid body.
pub enum ColliderShape {
    /// Sphere collider with a positive radius.
    Sphere {
        /// Collider radius in scene units.
        radius: f32,
    },
    /// Axis-aligned authored box represented by positive half-extents.
    Cuboid {
        /// Half the authored box size on each axis.
        half_extents: Vec3,
    },
}

#[derive(Clone, Debug, PartialEq)]
/// Stable link from one logical scene object to flattened render primitives.
pub enum ObjectBinding {
    /// Index into [`crate::Scene::spheres`].
    Sphere {
        /// Stable analytic-sphere index.
        index: usize,
    },
    /// Contiguous range in [`crate::Scene::triangles`].
    Triangles {
        /// First triangle generated for the object.
        start: usize,
        /// Number of triangles in the object.
        count: usize,
        /// Authored world-space pivot used for absolute rigid transforms.
        pivot: Vec3,
    },
}

#[derive(Clone, Debug, PartialEq)]
/// One validated rigid-body declaration independent of any physics library.
pub struct RigidBodyDeclaration {
    /// Static or dynamic ownership.
    pub body: RigidBodyKind,
    /// Collision geometry and dimensions.
    pub collider: ColliderShape,
    /// Dynamic mass in kilograms; absent for static bodies.
    pub mass: Option<f32>,
    /// Coulomb friction coefficient.
    pub friction: f32,
    /// Contact restitution coefficient in `[0, 1]`.
    pub restitution: f32,
    /// Initial world-space linear velocity in metres per second.
    pub initial_velocity: Vec3,
    /// Initial world-space angular velocity in radians per second.
    pub initial_angular_velocity: Vec3,
    /// Flattened render geometry controlled by this body.
    pub binding: ObjectBinding,
    /// Optional user-facing animation group attached to the source object.
    pub group: Option<String>,
}
