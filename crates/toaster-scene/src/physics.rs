//! Renderer-neutral physics declarations and stable render-object bindings.

use anyhow::{bail, Result};
use glam::Vec3;
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable renderer-neutral identity for a physics body or trigger.
pub struct PhysicsEntityId(String);

impl PhysicsEntityId {
    /// Validates and owns one nonempty identifier.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            bail!("physics entity id must not be empty");
        }
        Ok(Self(value))
    }

    /// Returns the identifier text used by scene declarations and events.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PhysicsEntityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Lifecycle phase for a physical contact between two bodies.
pub enum CollisionPhase {
    /// The pair became active during this tick.
    Started,
    /// The pair remained active through this nominal fixed tick.
    Stayed,
    /// The pair stopped touching during this tick.
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Lifecycle phase for an object overlapping a trigger sensor.
pub enum TriggerPhase {
    /// The object entered the trigger.
    Entered,
    /// The object exited the trigger.
    Exited,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Backend-neutral payload describing one physics event.
pub enum PhysicsEventKind {
    /// Physical contact between two canonically ordered object identities.
    Collision {
        /// Start, stay, or exit phase.
        phase: CollisionPhase,
        /// Lexicographically first body identity.
        object_a: PhysicsEntityId,
        /// Lexicographically second body identity.
        object_b: PhysicsEntityId,
    },
    /// Sensor overlap with fixed trigger and moving-object roles.
    Trigger {
        /// Enter or exit phase.
        phase: TriggerPhase,
        /// Trigger-zone identity.
        trigger: PhysicsEntityId,
        /// Dynamic or kinematic body identity.
        object: PhysicsEntityId,
    },
}

#[derive(Clone, Debug, PartialEq)]
/// One physics event positioned on the deterministic simulation timeline.
pub struct PhysicsEvent {
    /// Explicit loop identity from the evaluation request.
    pub loop_cycle: u64,
    /// One-based nominal tick completed when this event was observed.
    pub fixed_tick: u64,
    /// Wrapped scene-local event time in seconds.
    pub time_seconds: f32,
    /// Renderer-neutral event payload.
    pub kind: PhysicsEventKind,
}

#[derive(Clone, Debug, Default, PartialEq)]
/// Ordered physics-event delta produced by one scene evaluation.
pub struct PhysicsEventBatch {
    /// Whether consumers must clear prior event-driven state before applying this batch.
    pub reset: bool,
    /// Events from every nominal tick crossed by this evaluation.
    pub events: Vec<PhysicsEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Renderer-neutral selector for one class of physics events.
pub enum PhysicsEventMatcher {
    /// Unordered physical-contact matching with optional participant filters.
    Collision {
        /// Required contact lifecycle phase.
        phase: CollisionPhase,
        /// Optional concrete participant; absence represents a wildcard.
        object: Option<PhysicsEntityId>,
        /// Optional second concrete participant; absence represents a wildcard.
        other: Option<PhysicsEntityId>,
    },
    /// Trigger-overlap matching with optional trigger and object filters.
    Trigger {
        /// Required sensor lifecycle phase.
        phase: TriggerPhase,
        /// Optional concrete trigger; absence represents a wildcard.
        trigger: Option<PhysicsEntityId>,
        /// Optional concrete moving object; absence represents a wildcard.
        object: Option<PhysicsEntityId>,
    },
}

impl PhysicsEventMatcher {
    /// Returns whether one neutral event satisfies this selector.
    pub fn matches(&self, event: &PhysicsEventKind) -> bool {
        match (self, event) {
            (
                Self::Collision {
                    phase,
                    object,
                    other,
                },
                PhysicsEventKind::Collision {
                    phase: event_phase,
                    object_a,
                    object_b,
                },
            ) => {
                phase == event_phase
                    && match (object, other) {
                        (None, None) => true,
                        (Some(id), None) | (None, Some(id)) => id == object_a || id == object_b,
                        (Some(first), Some(second)) => {
                            (first == object_a && second == object_b)
                                || (first == object_b && second == object_a)
                        }
                    }
            }
            (
                Self::Trigger {
                    phase,
                    trigger,
                    object,
                },
                PhysicsEventKind::Trigger {
                    phase: event_phase,
                    trigger: event_trigger,
                    object: event_object,
                },
            ) => {
                phase == event_phase
                    && trigger.as_ref().is_none_or(|id| id == event_trigger)
                    && object.as_ref().is_none_or(|id| id == event_object)
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
/// Temporary material-index override caused by a matching event.
pub struct MaterialFlashDeclaration {
    /// Concrete rendered physics body to modify.
    pub target: PhysicsEntityId,
    /// Existing non-emissive scene material used during the flash.
    pub material_index: usize,
    /// Half-open flash lifetime in wrapped scene seconds.
    pub duration_seconds: f32,
}

#[derive(Clone, Debug, PartialEq)]
/// Declarative renderer-neutral response to matching physics events.
pub struct EventReactionDeclaration {
    /// Event selector evaluated against the ordered neutral event stream.
    pub event_matcher: PhysicsEventMatcher,
    /// Optional temporary material override.
    pub flash: Option<MaterialFlashDeclaration>,
    /// Optional named counter incremented once per matching event.
    pub counter: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
/// Named counter values accompanying one evaluated scene.
pub struct NamedCounterSnapshot {
    /// Counts reconstructed within the current loop cycle.
    pub loop_counts: BTreeMap<String, u64>,
    /// Counts observed once per fixed tick and loop cycle by this evaluator.
    pub session_counts: BTreeMap<String, u64>,
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
    /// Stable user-authored event identity.
    pub id: PhysicsEntityId,
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

#[derive(Clone, Debug, PartialEq)]
/// Invisible fixed sensor participating only in renderer-neutral trigger events.
pub struct TriggerDeclaration {
    /// Stable trigger identity in the shared physics namespace.
    pub id: PhysicsEntityId,
    /// Authored world-space sensor origin.
    pub center: Vec3,
    /// Sphere or cuboid sensor geometry.
    pub collider: ColliderShape,
}
