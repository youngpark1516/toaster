//! Renderer-neutral physics evaluation with replaceable backend implementations.

mod rapier_backend;

use anyhow::{bail, Context, Result};
use glam::Vec3;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Instant,
};
use toaster_scene::{
    animation_changes, apply_rigid_transform, EvaluatedScene, EvaluationRequest,
    NamedCounterSnapshot, ObjectBinding, PhysicsEntityId, PhysicsEventBatch, RigidTransform, Scene,
    SceneChanges, SceneEvaluationTimings, SceneEvaluator, TeleportVelocity,
};

pub use rapier_backend::RapierRigidBodyBackend;

#[derive(Clone, Copy, Debug, PartialEq)]
/// Backend-local request derived deterministically from a render frame.
pub struct PhysicsEvaluationRequest {
    /// Wrapped scene time in seconds.
    pub scene_time_seconds: f32,
    /// Latest completed nominal fixed tick at that time.
    pub fixed_tick: u64,
    /// Explicit zero-based animation/physics loop identity.
    pub loop_cycle: u64,
}

#[derive(Clone, Debug, PartialEq)]
/// Renderer-neutral change emitted by a physics backend.
pub enum PhysicsSceneUpdate {
    /// Sets one bound scene object to an absolute world-space pose.
    RigidTransform {
        /// Stable binding into the flattened renderer geometry.
        binding: ObjectBinding,
        /// Absolute pose without backend-specific math types.
        transform: RigidTransform,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Renderer-neutral velocity update applied with a body-state action.
pub enum PhysicsVelocityAction {
    /// Retain the backend's current linear and angular velocity.
    Preserve,
    /// Replace both velocities with explicit world-space values.
    Set {
        /// Linear velocity in metres per second.
        linear: Vec3,
        /// Angular velocity in radians per second.
        angular: Vec3,
    },
}

#[derive(Clone, Debug, PartialEq)]
/// Renderer-neutral command that replaces one dynamic body's state.
pub enum PhysicsBodyAction {
    /// Sets an absolute pose and velocity policy, clears forces, and wakes the body.
    SetState {
        /// Stable scene physics identity.
        target: PhysicsEntityId,
        /// Absolute world-space body pose.
        transform: RigidTransform,
        /// Explicit or preserving velocity behavior.
        velocity: PhysicsVelocityAction,
    },
}

#[derive(Clone, Debug, PartialEq)]
/// Backend result before it is composed into an evaluated render scene.
pub struct EvaluatedPhysicsState {
    /// Absolute renderer-neutral updates.
    pub updates: Vec<PhysicsSceneUpdate>,
    /// Renderer data categories invalidated by the updates.
    pub changes: SceneChanges,
    /// Ordered neutral events crossed while reaching the requested tick.
    pub physics_events: PhysicsEventBatch,
}

/// Replaceable physics simulation backend hidden from renderer crates.
pub trait PhysicsBackend {
    /// Restores the authored initial world state.
    fn reset(&mut self) -> Result<()>;

    /// Evaluates the backend at one fixed tick and returns neutral scene updates.
    fn evaluate(&mut self, request: PhysicsEvaluationRequest) -> Result<EvaluatedPhysicsState>;

    /// Applies renderer-neutral body-state changes between nominal fixed ticks.
    fn apply_actions(&mut self, actions: &[PhysicsBodyAction]) -> Result<()>;
}

/// Composes built-in animation and an optional physics backend from an immutable scene.
pub struct PhysicsSceneEvaluator {
    source: Scene,
    backend: Option<Box<dyn PhysicsBackend>>,
    evaluated_once: bool,
    last_physics_request: Option<PhysicsEvaluationRequest>,
    reactions: EventReactionState,
}

#[derive(Clone, Copy, Debug)]
struct ActiveFlash {
    material_index: usize,
    expires_at_seconds: f64,
}

#[derive(Default)]
struct EventReactionState {
    loop_counts: BTreeMap<String, u64>,
    session_counts: BTreeMap<String, u64>,
    highest_tick_by_loop: HashMap<u64, u64>,
    active_flashes: HashMap<PhysicsEntityId, ActiveFlash>,
    output_flash_materials: HashMap<PhysicsEntityId, Option<usize>>,
}

impl EventReactionState {
    fn new(source: &Scene) -> Result<Self> {
        let mut state = Self::default();
        for reaction in &source.event_reactions {
            if let Some(counter) = &reaction.counter {
                state.loop_counts.entry(counter.clone()).or_insert(0);
                state.session_counts.entry(counter.clone()).or_insert(0);
            }
            if let Some(flash) = &reaction.flash {
                state
                    .output_flash_materials
                    .insert(flash.target.clone(), None);
            }
        }
        Ok(state)
    }

    fn process_events(
        &mut self,
        source: &Scene,
        request: PhysicsEvaluationRequest,
        events: &PhysicsEventBatch,
    ) -> Result<Vec<PhysicsBodyAction>> {
        if events.reset {
            self.active_flashes.clear();
            self.loop_counts.values_mut().for_each(|count| *count = 0);
        }

        let previous_highest_tick = self
            .highest_tick_by_loop
            .get(&request.loop_cycle)
            .copied()
            .unwrap_or(0);
        let mut actions = Vec::new();
        let mut acted_targets = HashSet::new();
        for event in &events.events {
            let new_session_tick = event.fixed_tick > previous_highest_tick;
            for reaction in &source.event_reactions {
                if !reaction.event_matcher.matches(&event.kind) {
                    continue;
                }
                if let Some(counter) = &reaction.counter {
                    increment_counter(&mut self.loop_counts, counter)?;
                    if new_session_tick {
                        increment_counter(&mut self.session_counts, counter)?;
                    }
                }
                if let Some(flash) = &reaction.flash {
                    self.active_flashes.insert(
                        flash.target.clone(),
                        ActiveFlash {
                            material_index: flash.material_index,
                            expires_at_seconds: f64::from(event.time_seconds)
                                + f64::from(flash.duration_seconds),
                        },
                    );
                }
                if let Some(reset) = &reaction.reset {
                    let body = source
                        .rigid_bodies
                        .iter()
                        .find(|body| body.id == reset.target)
                        .context("reset action target has no rigid-body declaration")?;
                    actions.push(PhysicsBodyAction::SetState {
                        target: reset.target.clone(),
                        transform: body.initial_transform,
                        velocity: PhysicsVelocityAction::Set {
                            linear: body.initial_velocity,
                            angular: body.initial_angular_velocity,
                        },
                    });
                    acted_targets.insert(reset.target.clone());
                    tracing::debug!(
                        loop_cycle = event.loop_cycle,
                        fixed_tick = event.fixed_tick,
                        action = "reset",
                        target = %reset.target,
                        "physics event action"
                    );
                }
                if let Some(teleport) = &reaction.teleport {
                    let spawn = source
                        .spawn_points
                        .get(teleport.spawn_point_index)
                        .context("teleport action spawn-point index is out of range")?;
                    actions.push(PhysicsBodyAction::SetState {
                        target: teleport.target.clone(),
                        transform: spawn.transform,
                        velocity: match teleport.velocity {
                            TeleportVelocity::Clear => PhysicsVelocityAction::Set {
                                linear: Vec3::ZERO,
                                angular: Vec3::ZERO,
                            },
                            TeleportVelocity::Preserve => PhysicsVelocityAction::Preserve,
                        },
                    });
                    acted_targets.insert(teleport.target.clone());
                    tracing::debug!(
                        loop_cycle = event.loop_cycle,
                        fixed_tick = event.fixed_tick,
                        action = "teleport",
                        target = %teleport.target,
                        spawn = %spawn.name,
                        velocity = ?teleport.velocity,
                        "physics event action"
                    );
                }
            }
        }
        for target in acted_targets {
            self.active_flashes.remove(&target);
        }
        self.highest_tick_by_loop
            .entry(request.loop_cycle)
            .and_modify(|tick| *tick = (*tick).max(request.fixed_tick))
            .or_insert(request.fixed_tick);

        Ok(actions)
    }

    fn apply_materials(
        &mut self,
        source: &Scene,
        scene: &mut Scene,
        scene_time_seconds: f32,
    ) -> Result<SceneChanges> {
        let scene_time = f64::from(scene_time_seconds);
        self.active_flashes
            .retain(|_, flash| scene_time < flash.expires_at_seconds);

        let mut changes = SceneChanges::default();
        for (target, previous_material) in &mut self.output_flash_materials {
            let body = source
                .rigid_bodies
                .iter()
                .find(|body| body.id == *target)
                .context("material flash target has no rigid-body declaration")?;
            let desired_material = self
                .active_flashes
                .get(target)
                .map(|flash| flash.material_index);
            if let Some(material_index) = desired_material {
                apply_binding_material(scene, &body.binding, material_index)?;
            }
            if *previous_material != desired_material {
                match body.binding {
                    ObjectBinding::Sphere { .. } => changes.spheres = true,
                    ObjectBinding::Triangles { .. } => changes.triangles = true,
                }
                *previous_material = desired_material;
            }
        }
        Ok(changes)
    }

    #[cfg(test)]
    fn evaluate(
        &mut self,
        source: &Scene,
        scene: &mut Scene,
        request: PhysicsEvaluationRequest,
        events: &PhysicsEventBatch,
    ) -> Result<SceneChanges> {
        self.process_events(source, request, events)?;
        self.apply_materials(source, scene, request.scene_time_seconds)
    }

    fn snapshot(&self) -> NamedCounterSnapshot {
        NamedCounterSnapshot {
            loop_counts: self.loop_counts.clone(),
            session_counts: self.session_counts.clone(),
        }
    }
}

fn increment_counter(counters: &mut BTreeMap<String, u64>, name: &str) -> Result<()> {
    let counter = counters
        .get_mut(name)
        .context("event reaction counter was not initialized")?;
    *counter = counter
        .checked_add(1)
        .context("event reaction counter overflowed")?;
    Ok(())
}

fn apply_binding_material(
    scene: &mut Scene,
    binding: &ObjectBinding,
    material_index: usize,
) -> Result<()> {
    match *binding {
        ObjectBinding::Sphere { index } => {
            scene
                .spheres
                .get_mut(index)
                .context("material flash sphere binding is out of range")?
                .material_index = material_index;
        }
        ObjectBinding::Triangles { start, count, .. } => {
            let triangles = scene
                .triangles
                .get_mut(start..start.saturating_add(count))
                .context("material flash triangle binding is out of range")?;
            for triangle in triangles {
                triangle.material_index = material_index;
            }
        }
    }
    Ok(())
}

impl PhysicsSceneEvaluator {
    /// Builds the default backend selected by the scene's simulation domain.
    pub fn new(source: Scene) -> Result<Self> {
        let backend = create_backend(&source)?;
        let reactions = EventReactionState::new(&source)?;
        Ok(Self {
            source,
            backend,
            evaluated_once: false,
            last_physics_request: None,
            reactions,
        })
    }

    /// Installs a caller-supplied backend, primarily for alternative solvers and tests.
    pub fn with_backend(source: Scene, backend: Box<dyn PhysicsBackend>) -> Result<Self> {
        if !source.physics.is_some_and(|settings| settings.enabled) {
            bail!("a custom physics backend requires physics.enabled to be true");
        }
        let reactions = EventReactionState::new(&source)?;
        Ok(Self {
            source,
            backend: Some(backend),
            evaluated_once: false,
            last_physics_request: None,
            reactions,
        })
    }

    fn physics_request(
        settings: toaster_scene::PhysicsSettings,
        request: EvaluationRequest,
    ) -> Result<PhysicsEvaluationRequest> {
        if !request.time_seconds.is_finite() || request.time_seconds < 0.0 {
            bail!("scene evaluation time must be finite and non-negative");
        }
        // Decimal JSON encodings of 1/60 should land on their intended tick.
        let tolerance = settings.timestep * 1.0e-5;
        let tick = ((request.time_seconds + tolerance) / settings.timestep).floor();
        if !tick.is_finite() || tick < 0.0 || tick > u64::MAX as f32 {
            bail!("physics fixed tick is outside the supported range");
        }
        Ok(PhysicsEvaluationRequest {
            scene_time_seconds: request.time_seconds,
            fixed_tick: tick as u64,
            loop_cycle: request.loop_cycle,
        })
    }
}

/// Selects an implementation for the declared simulation domain without exposing it to renderers.
fn create_backend(source: &Scene) -> Result<Option<Box<dyn PhysicsBackend>>> {
    match source.physics {
        Some(settings) if settings.enabled => match settings.physics_type {
            toaster_scene::PhysicsType::RigidBody => {
                Ok(Some(Box::new(RapierRigidBodyBackend::new(source)?)))
            }
        },
        _ => Ok(None),
    }
}

impl SceneEvaluator for PhysicsSceneEvaluator {
    fn source_scene(&self) -> &Scene {
        &self.source
    }

    fn is_time_varying(&self) -> bool {
        !self.source.animation.tracks.is_empty()
            || self.source.physics.is_some_and(|settings| settings.enabled)
    }

    fn evaluate(&mut self, request: EvaluationRequest) -> Result<EvaluatedScene> {
        let animation_start = Instant::now();
        let mut scene = self.source.evaluate_at(request.time_seconds)?;
        let animation_evaluation = animation_start.elapsed();
        let mut changes = animation_changes(&self.source);
        let mut physics_evaluation = Default::default();
        let mut geometry_update = Default::default();
        let mut physics_events = PhysicsEventBatch::default();
        let has_motion_actions = self
            .source
            .event_reactions
            .iter()
            .any(|reaction| reaction.reset.is_some() || reaction.teleport.is_some());
        if let Some(backend) = &mut self.backend {
            let settings = self
                .source
                .physics
                .context("active physics evaluator requires physics settings")?;
            let backend_request = Self::physics_request(settings, request)?;
            let physics_start = Instant::now();
            let reset = self.last_physics_request.is_some_and(|previous| {
                previous.loop_cycle != backend_request.loop_cycle
                    || backend_request.fixed_tick < previous.fixed_tick
                    || backend_request.scene_time_seconds < previous.scene_time_seconds
            });
            if reset {
                backend.reset()?;
            }
            let mut accumulated_events = PhysicsEventBatch {
                reset,
                events: Vec::new(),
            };
            let mut accumulated_changes = SceneChanges::default();
            let mut final_state;
            if has_motion_actions {
                let start_tick = if reset {
                    0
                } else {
                    self.last_physics_request
                        .filter(|previous| previous.loop_cycle == backend_request.loop_cycle)
                        .map_or(0, |previous| previous.fixed_tick)
                };
                if start_tick < backend_request.fixed_tick {
                    for fixed_tick in start_tick + 1..=backend_request.fixed_tick {
                        let tick_request = PhysicsEvaluationRequest {
                            scene_time_seconds: (fixed_tick as f64 * settings.timestep as f64)
                                as f32,
                            fixed_tick,
                            loop_cycle: backend_request.loop_cycle,
                        };
                        let mut tick_state = backend.evaluate(tick_request)?;
                        if fixed_tick == start_tick + 1 {
                            tick_state.physics_events.reset |= reset;
                        }
                        let actions = self.reactions.process_events(
                            &self.source,
                            tick_request,
                            &tick_state.physics_events,
                        )?;
                        backend.apply_actions(&actions)?;
                        accumulated_events.reset |= tick_state.physics_events.reset;
                        accumulated_events
                            .events
                            .extend(tick_state.physics_events.events);
                        accumulated_changes = accumulated_changes.union(tick_state.changes);
                    }
                    // Re-reading the same tick returns the post-action transforms without
                    // integrating or duplicating events.
                    final_state = backend.evaluate(backend_request)?;
                    if !final_state.physics_events.events.is_empty() {
                        bail!("physics backend duplicated events while reading action state");
                    }
                } else {
                    final_state = backend.evaluate(backend_request)?;
                    final_state.physics_events.reset |= reset;
                    let actions = self.reactions.process_events(
                        &self.source,
                        backend_request,
                        &final_state.physics_events,
                    )?;
                    backend.apply_actions(&actions)?;
                    accumulated_events.reset |= final_state.physics_events.reset;
                    accumulated_events
                        .events
                        .append(&mut final_state.physics_events.events);
                    accumulated_changes = accumulated_changes.union(final_state.changes);
                    if !actions.is_empty() {
                        final_state = backend.evaluate(backend_request)?;
                        if !final_state.physics_events.events.is_empty() {
                            bail!("physics backend duplicated events while reading action state");
                        }
                    }
                }
                final_state.changes = final_state.changes.union(accumulated_changes);
                final_state.physics_events = accumulated_events;
            } else {
                final_state = backend.evaluate(backend_request)?;
                final_state.physics_events.reset |= reset;
                let actions = self.reactions.process_events(
                    &self.source,
                    backend_request,
                    &final_state.physics_events,
                )?;
                debug_assert!(actions.is_empty());
            }
            physics_evaluation = physics_start.elapsed();
            self.last_physics_request = Some(backend_request);
            if final_state.physics_events.reset {
                tracing::debug!(
                    loop_cycle = backend_request.loop_cycle,
                    fixed_tick = backend_request.fixed_tick,
                    "reset physics event timeline before replay"
                );
            }
            for event in &final_state.physics_events.events {
                tracing::debug!(
                    loop_cycle = event.loop_cycle,
                    fixed_tick = event.fixed_tick,
                    event_time_seconds = event.time_seconds as f64,
                    event = ?event.kind,
                    "physics event"
                );
            }
            let geometry_start = Instant::now();
            for update in final_state.updates {
                match update {
                    PhysicsSceneUpdate::RigidTransform { binding, transform } => {
                        apply_rigid_transform(&self.source, &mut scene, &binding, transform)?;
                    }
                }
            }
            let reaction_changes = self.reactions.apply_materials(
                &self.source,
                &mut scene,
                backend_request.scene_time_seconds,
            )?;
            geometry_update = geometry_start.elapsed();
            changes = changes.union(final_state.changes).union(reaction_changes);
            physics_events = final_state.physics_events;
        }
        if !self.evaluated_once {
            changes = SceneChanges::all();
        }
        self.evaluated_once = true;
        Ok(EvaluatedScene {
            scene,
            changes,
            timings: SceneEvaluationTimings {
                animation_evaluation,
                physics_evaluation,
                geometry_update,
            },
            physics_events,
            counters: self.reactions.snapshot(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use std::{cell::RefCell, rc::Rc};
    use toaster_scene::{
        Animation, AnimationTarget, AnimationTrack, Background, CameraSettings, ColliderShape,
        CollisionPhase, EventReactionDeclaration, Interpolation, Material,
        MaterialFlashDeclaration, PhysicsEntityId, PhysicsEvent, PhysicsEventKind,
        PhysicsEventMatcher, PhysicsSettings, PhysicsType, RenderSettings, ResetBodyDeclaration,
        RigidBodyDeclaration, RigidBodyKind, SpawnPointDeclaration, Sphere,
        TeleportBodyDeclaration, TeleportVelocity, TranslationKeyframe, Triangle,
        TriangleAttributes, TriggerDeclaration, TriggerPhase,
    };

    pub(crate) fn base_scene() -> Scene {
        let sphere_binding = ObjectBinding::Sphere { index: 0 };
        let floor_binding = ObjectBinding::Triangles {
            start: 0,
            count: 1,
            pivot: Vec3::new(0.0, -0.1, 0.0),
        };
        Scene {
            camera: CameraSettings {
                position: Vec3::new(0.0, 2.0, 5.0),
                look_at: Vec3::Y,
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
            materials: vec![Material::Diffuse { albedo: Vec3::ONE }],
            spheres: vec![Sphere {
                center: Vec3::new(0.0, 3.0, 0.0),
                radius: 0.5,
                material_index: 0,
                group: Some("ball".into()),
            }],
            triangles: vec![Triangle {
                vertices: [
                    Vec3::new(-5.0, 0.0, -5.0),
                    Vec3::new(5.0, 0.0, -5.0),
                    Vec3::new(5.0, 0.0, 5.0),
                ],
                material_index: 0,
                group: Some("floor".into()),
            }],
            triangle_attributes: vec![TriangleAttributes::default()],
            textures: Vec::new(),
            environment: None,
            animation: Default::default(),
            physics: Some(PhysicsSettings {
                enabled: true,
                physics_type: PhysicsType::RigidBody,
                gravity: Vec3::new(0.0, -9.81, 0.0),
                timestep: 1.0 / 60.0,
                substeps: 1,
            }),
            rigid_bodies: vec![
                RigidBodyDeclaration {
                    id: PhysicsEntityId::new("ball").unwrap(),
                    body: RigidBodyKind::Dynamic,
                    collider: ColliderShape::Sphere { radius: 0.5 },
                    mass: Some(1.0),
                    friction: 0.5,
                    restitution: 0.0,
                    initial_velocity: Vec3::ZERO,
                    initial_angular_velocity: Vec3::ZERO,
                    initial_transform: RigidTransform {
                        translation: Vec3::new(0.0, 3.0, 0.0),
                        rotation: glam::Quat::IDENTITY,
                    },
                    binding: sphere_binding,
                    group: Some("ball".into()),
                },
                RigidBodyDeclaration {
                    id: PhysicsEntityId::new("floor").unwrap(),
                    body: RigidBodyKind::Static,
                    collider: ColliderShape::Cuboid {
                        half_extents: Vec3::new(5.0, 0.1, 5.0),
                    },
                    mass: None,
                    friction: 0.8,
                    restitution: 0.0,
                    initial_velocity: Vec3::ZERO,
                    initial_angular_velocity: Vec3::ZERO,
                    initial_transform: RigidTransform {
                        translation: Vec3::new(0.0, -0.1, 0.0),
                        rotation: glam::Quat::IDENTITY,
                    },
                    binding: floor_binding,
                    group: Some("floor".into()),
                },
            ],
            triggers: Vec::new(),
            spawn_points: Vec::new(),
            event_reactions: Vec::new(),
        }
    }

    fn request(time_seconds: f32, loop_cycle: u64) -> EvaluationRequest {
        EvaluationRequest {
            time_seconds,
            loop_cycle,
        }
    }

    fn kinematic_scene() -> Scene {
        let mut scene = base_scene();
        scene.spheres[0].center = Vec3::new(0.0, 0.5, 0.0);
        scene.rigid_bodies[0].initial_transform.translation = scene.spheres[0].center;
        scene.triangles[0].group = Some("platform".into());
        scene.rigid_bodies[1].id = PhysicsEntityId::new("platform").unwrap();
        scene.rigid_bodies[1].body = RigidBodyKind::Kinematic;
        scene.rigid_bodies[1].group = Some("platform".into());
        scene.physics.as_mut().unwrap().substeps = 2;
        scene.animation = Animation {
            tracks: vec![AnimationTrack::Translation {
                target: AnimationTarget::Group {
                    name: "platform".into(),
                },
                interpolation: Interpolation::Linear,
                keyframes: vec![
                    TranslationKeyframe {
                        time: 0.0,
                        value: Vec3::ZERO,
                    },
                    TranslationKeyframe {
                        time: 1.0,
                        value: Vec3::Y,
                    },
                ],
            }],
        };
        scene
    }

    fn trigger_scene() -> Scene {
        let mut scene = base_scene();
        scene.physics.as_mut().unwrap().gravity = Vec3::ZERO;
        scene.spheres[0].center = Vec3::new(-2.0, 1.0, 0.0);
        scene.rigid_bodies[0].initial_transform.translation = scene.spheres[0].center;
        scene.spheres[0].radius = 0.25;
        scene.rigid_bodies[0].collider = ColliderShape::Sphere { radius: 0.25 };
        scene.rigid_bodies[0].initial_velocity = Vec3::new(2.0, 0.0, 0.0);
        scene.triggers = vec![TriggerDeclaration {
            id: PhysicsEntityId::new("zone").unwrap(),
            center: Vec3::Y,
            collider: ColliderShape::Sphere { radius: 0.5 },
        }];
        scene
    }

    fn reaction_scene() -> Scene {
        let mut scene = base_scene();
        scene.materials.push(Material::Diffuse {
            albedo: Vec3::new(1.0, 1.0, 0.0),
        });
        scene.event_reactions = vec![EventReactionDeclaration {
            event_matcher: PhysicsEventMatcher::Collision {
                phase: CollisionPhase::Started,
                object: Some(PhysicsEntityId::new("ball").unwrap()),
                other: Some(PhysicsEntityId::new("floor").unwrap()),
            },
            flash: Some(MaterialFlashDeclaration {
                target: PhysicsEntityId::new("ball").unwrap(),
                material_index: 1,
                duration_seconds: 0.2,
            }),
            counter: Some("hits".into()),
            reset: None,
            teleport: None,
        }];
        scene
    }

    fn action_scene(reset: bool, velocity: TeleportVelocity) -> Scene {
        let mut scene = trigger_scene();
        scene.physics.as_mut().unwrap().timestep = 0.1;
        scene.spheres[0].center = Vec3::new(-1.0, 1.0, 0.0);
        scene.rigid_bodies[0].initial_transform.translation = scene.spheres[0].center;
        scene.rigid_bodies[0].initial_velocity = Vec3::new(5.0, 0.0, 0.0);
        scene.spawn_points.push(SpawnPointDeclaration {
            name: "destination".into(),
            transform: RigidTransform {
                translation: Vec3::new(3.0, 2.0, 0.0),
                rotation: Quat::from_rotation_y(0.5),
            },
        });
        scene.event_reactions.push(EventReactionDeclaration {
            event_matcher: PhysicsEventMatcher::Trigger {
                phase: TriggerPhase::Entered,
                trigger: Some(PhysicsEntityId::new("zone").unwrap()),
                object: Some(PhysicsEntityId::new("ball").unwrap()),
            },
            flash: None,
            counter: Some(if reset { "resets" } else { "teleports" }.into()),
            reset: reset.then(|| ResetBodyDeclaration {
                target: PhysicsEntityId::new("ball").unwrap(),
            }),
            teleport: (!reset).then(|| TeleportBodyDeclaration {
                target: PhysicsEntityId::new("ball").unwrap(),
                spawn_point_index: 0,
                velocity,
            }),
        });
        scene
    }

    fn collision_event(loop_cycle: u64, fixed_tick: u64, time_seconds: f32) -> PhysicsEvent {
        PhysicsEvent {
            loop_cycle,
            fixed_tick,
            time_seconds,
            kind: PhysicsEventKind::Collision {
                phase: CollisionPhase::Started,
                object_a: PhysicsEntityId::new("ball").unwrap(),
                object_b: PhysicsEntityId::new("floor").unwrap(),
            },
        }
    }

    #[test]
    fn sphere_falls_and_collides_with_static_floor() {
        let mut evaluator = PhysicsSceneEvaluator::new(base_scene()).unwrap();
        let falling = evaluator.evaluate(request(0.5, 0)).unwrap();
        assert!(falling.scene.spheres[0].center.y < 3.0);
        assert_eq!(falling.scene.spheres[0].radius, 0.5);
        assert_eq!(falling.scene.spheres[0].material_index, 0);
        assert_eq!(falling.scene.spheres[0].group.as_deref(), Some("ball"));
        let settled = evaluator.evaluate(request(3.0, 0)).unwrap();
        assert!(settled.scene.spheres[0].center.y >= 0.49);
        assert!(settled.scene.spheres[0].center.y < 0.6);
        assert_eq!(evaluator.source_scene().spheres[0].center.y, 3.0);
    }

    #[test]
    fn collision_events_include_started_stayed_and_canonical_ids() {
        let mut scene = base_scene();
        scene.rigid_bodies.reverse();
        let mut evaluator = PhysicsSceneEvaluator::new(scene).unwrap();
        let evaluated = evaluator.evaluate(request(1.5, 0)).unwrap();
        let phases = evaluated
            .physics_events
            .events
            .iter()
            .filter_map(|event| match &event.kind {
                PhysicsEventKind::Collision {
                    phase,
                    object_a,
                    object_b,
                } if object_a.as_str() == "ball" && object_b.as_str() == "floor" => Some(*phase),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(phases.contains(&CollisionPhase::Started));
        assert!(phases.contains(&CollisionPhase::Stayed));
        assert!(evaluated
            .physics_events
            .events
            .windows(2)
            .all(|events| events[0].fixed_tick <= events[1].fixed_tick));

        let same_tick = evaluator.evaluate(request(1.5, 0)).unwrap();
        assert!(same_tick.physics_events.events.is_empty());
    }

    #[test]
    fn reaction_flash_activates_extends_expires_and_preserves_source() {
        let source = reaction_scene();
        let mut state = EventReactionState::new(&source).unwrap();
        let mut evaluated = source.clone();
        let started = PhysicsEventBatch {
            reset: false,
            events: vec![collision_event(0, 6, 0.1)],
        };
        let changes = state
            .evaluate(
                &source,
                &mut evaluated,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.15,
                    fixed_tick: 9,
                    loop_cycle: 0,
                },
                &started,
            )
            .unwrap();
        assert!(changes.spheres);
        assert_eq!(evaluated.spheres[0].material_index, 1);
        assert_eq!(source.spheres[0].material_index, 0);
        assert_eq!(state.snapshot().loop_counts["hits"], 1);

        let extended = PhysicsEventBatch {
            reset: false,
            events: vec![collision_event(0, 12, 0.2)],
        };
        let mut extended_scene = source.clone();
        let changes = state
            .evaluate(
                &source,
                &mut extended_scene,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.35,
                    fixed_tick: 21,
                    loop_cycle: 0,
                },
                &extended,
            )
            .unwrap();
        assert!(!changes.spheres);
        assert_eq!(extended_scene.spheres[0].material_index, 1);

        let mut expired_scene = source.clone();
        let changes = state
            .evaluate(
                &source,
                &mut expired_scene,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.4,
                    fixed_tick: 24,
                    loop_cycle: 0,
                },
                &PhysicsEventBatch::default(),
            )
            .unwrap();
        assert!(changes.spheres);
        assert_eq!(expired_scene.spheres[0].material_index, 0);
    }

    #[test]
    fn reaction_flash_marks_bound_triangle_material_changes() {
        let mut source = reaction_scene();
        source.event_reactions.push(EventReactionDeclaration {
            event_matcher: PhysicsEventMatcher::Collision {
                phase: CollisionPhase::Started,
                object: Some(PhysicsEntityId::new("ball").unwrap()),
                other: Some(PhysicsEntityId::new("floor").unwrap()),
            },
            flash: Some(MaterialFlashDeclaration {
                target: PhysicsEntityId::new("floor").unwrap(),
                material_index: 1,
                duration_seconds: 0.2,
            }),
            counter: None,
            reset: None,
            teleport: None,
        });
        let mut state = EventReactionState::new(&source).unwrap();
        let mut evaluated = source.clone();
        let changes = state
            .evaluate(
                &source,
                &mut evaluated,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.15,
                    fixed_tick: 9,
                    loop_cycle: 0,
                },
                &PhysicsEventBatch {
                    reset: false,
                    events: vec![collision_event(0, 6, 0.1)],
                },
            )
            .unwrap();

        assert!(changes.spheres);
        assert!(changes.triangles);
        assert_eq!(evaluated.triangles[0].material_index, 1);
        assert_eq!(source.triangles[0].material_index, 0);
    }

    #[test]
    fn reaction_flash_restores_each_authored_material_in_mesh_range() {
        let mut source = reaction_scene();
        source.materials.push(Material::Diffuse { albedo: Vec3::X });
        let mut second = source.triangles[0].clone();
        second.material_index = 2;
        source.triangles.push(second);
        source
            .triangle_attributes
            .push(TriangleAttributes::default());
        source.rigid_bodies[1].binding = ObjectBinding::Triangles {
            start: 0,
            count: 2,
            pivot: Vec3::new(0.0, -0.1, 0.0),
        };
        source.event_reactions.push(EventReactionDeclaration {
            event_matcher: PhysicsEventMatcher::Collision {
                phase: CollisionPhase::Started,
                object: Some(PhysicsEntityId::new("ball").unwrap()),
                other: Some(PhysicsEntityId::new("floor").unwrap()),
            },
            flash: Some(MaterialFlashDeclaration {
                target: PhysicsEntityId::new("floor").unwrap(),
                material_index: 1,
                duration_seconds: 0.2,
            }),
            counter: None,
            reset: None,
            teleport: None,
        });

        let mut state = EventReactionState::new(&source).unwrap();
        let mut flashed = source.clone();
        state
            .evaluate(
                &source,
                &mut flashed,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.15,
                    fixed_tick: 9,
                    loop_cycle: 0,
                },
                &PhysicsEventBatch {
                    reset: false,
                    events: vec![collision_event(0, 6, 0.1)],
                },
            )
            .unwrap();
        assert_eq!(flashed.triangles[0].material_index, 1);
        assert_eq!(flashed.triangles[1].material_index, 1);

        let mut restored = source.clone();
        let changes = state
            .evaluate(
                &source,
                &mut restored,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.31,
                    fixed_tick: 19,
                    loop_cycle: 0,
                },
                &PhysicsEventBatch::default(),
            )
            .unwrap();
        assert!(changes.triangles);
        assert_eq!(restored.triangles[0].material_index, 0);
        assert_eq!(restored.triangles[1].material_index, 2);
    }

    #[test]
    fn loop_reset_clears_boundary_flash_and_replay_can_recreate_it() {
        let source = reaction_scene();
        let mut state = EventReactionState::new(&source).unwrap();
        let mut end_of_loop = source.clone();
        state
            .evaluate(
                &source,
                &mut end_of_loop,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.99,
                    fixed_tick: 59,
                    loop_cycle: 0,
                },
                &PhysicsEventBatch {
                    reset: false,
                    events: vec![collision_event(0, 57, 0.95)],
                },
            )
            .unwrap();
        assert_eq!(end_of_loop.spheres[0].material_index, 1);

        let mut next_loop = source.clone();
        state
            .evaluate(
                &source,
                &mut next_loop,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.01,
                    fixed_tick: 0,
                    loop_cycle: 1,
                },
                &PhysicsEventBatch {
                    reset: true,
                    events: Vec::new(),
                },
            )
            .unwrap();
        assert_eq!(next_loop.spheres[0].material_index, 0);
        assert_eq!(state.snapshot().loop_counts["hits"], 0);
        assert_eq!(state.snapshot().session_counts["hits"], 1);

        let mut replayed = source.clone();
        state
            .evaluate(
                &source,
                &mut replayed,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.16,
                    fixed_tick: 9,
                    loop_cycle: 1,
                },
                &PhysicsEventBatch {
                    reset: false,
                    events: vec![collision_event(1, 6, 0.1)],
                },
            )
            .unwrap();
        assert_eq!(replayed.spheres[0].material_index, 1);
        assert_eq!(state.snapshot().session_counts["hits"], 2);
    }

    #[test]
    fn rewind_rebuilds_loop_counters_without_inflating_session_counts() {
        let source = reaction_scene();
        let mut state = EventReactionState::new(&source).unwrap();
        let mut evaluated = source.clone();
        state
            .evaluate(
                &source,
                &mut evaluated,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.2,
                    fixed_tick: 2,
                    loop_cycle: 0,
                },
                &PhysicsEventBatch {
                    reset: false,
                    events: vec![collision_event(0, 1, 0.1), collision_event(0, 2, 0.2)],
                },
            )
            .unwrap();
        assert_eq!(state.snapshot().loop_counts["hits"], 2);
        assert_eq!(state.snapshot().session_counts["hits"], 2);

        let mut rewound = source.clone();
        state
            .evaluate(
                &source,
                &mut rewound,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.1,
                    fixed_tick: 1,
                    loop_cycle: 0,
                },
                &PhysicsEventBatch {
                    reset: true,
                    events: vec![collision_event(0, 1, 0.1)],
                },
            )
            .unwrap();
        assert_eq!(state.snapshot().loop_counts["hits"], 1);
        assert_eq!(state.snapshot().session_counts["hits"], 2);

        let mut forward = source.clone();
        state
            .evaluate(
                &source,
                &mut forward,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.3,
                    fixed_tick: 3,
                    loop_cycle: 0,
                },
                &PhysicsEventBatch {
                    reset: false,
                    events: vec![collision_event(0, 2, 0.2), collision_event(0, 3, 0.3)],
                },
            )
            .unwrap();
        assert_eq!(state.snapshot().loop_counts["hits"], 3);
        assert_eq!(state.snapshot().session_counts["hits"], 3);
    }

    #[test]
    fn stayed_and_trigger_counters_increment_only_matching_rules() {
        let mut source = trigger_scene();
        source.event_reactions = vec![
            EventReactionDeclaration {
                event_matcher: PhysicsEventMatcher::Collision {
                    phase: CollisionPhase::Started,
                    object: None,
                    other: None,
                },
                flash: None,
                counter: Some("starts".into()),
                reset: None,
                teleport: None,
            },
            EventReactionDeclaration {
                event_matcher: PhysicsEventMatcher::Collision {
                    phase: CollisionPhase::Stayed,
                    object: None,
                    other: None,
                },
                flash: None,
                counter: Some("stays".into()),
                reset: None,
                teleport: None,
            },
            EventReactionDeclaration {
                event_matcher: PhysicsEventMatcher::Trigger {
                    phase: TriggerPhase::Entered,
                    trigger: Some(PhysicsEntityId::new("zone").unwrap()),
                    object: None,
                },
                flash: None,
                counter: Some("entries".into()),
                reset: None,
                teleport: None,
            },
        ];
        let mut state = EventReactionState::new(&source).unwrap();
        let mut evaluated = source.clone();
        state
            .evaluate(
                &source,
                &mut evaluated,
                PhysicsEvaluationRequest {
                    scene_time_seconds: 0.2,
                    fixed_tick: 2,
                    loop_cycle: 0,
                },
                &PhysicsEventBatch {
                    reset: false,
                    events: vec![
                        collision_event(0, 1, 0.1),
                        PhysicsEvent {
                            loop_cycle: 0,
                            fixed_tick: 2,
                            time_seconds: 0.2,
                            kind: PhysicsEventKind::Collision {
                                phase: CollisionPhase::Stayed,
                                object_a: PhysicsEntityId::new("ball").unwrap(),
                                object_b: PhysicsEntityId::new("floor").unwrap(),
                            },
                        },
                        PhysicsEvent {
                            loop_cycle: 0,
                            fixed_tick: 2,
                            time_seconds: 0.2,
                            kind: PhysicsEventKind::Trigger {
                                phase: TriggerPhase::Entered,
                                trigger: PhysicsEntityId::new("zone").unwrap(),
                                object: PhysicsEntityId::new("ball").unwrap(),
                            },
                        },
                    ],
                },
            )
            .unwrap();
        let snapshot = state.snapshot();
        assert_eq!(snapshot.loop_counts["starts"], 1);
        assert_eq!(snapshot.loop_counts["stays"], 1);
        assert_eq!(snapshot.loop_counts["entries"], 1);
    }

    #[test]
    fn disabled_physics_exposes_declared_zero_counters_without_flashing() {
        let mut source = reaction_scene();
        source.physics.as_mut().unwrap().enabled = false;
        let mut evaluator = PhysicsSceneEvaluator::new(source).unwrap();
        let evaluated = evaluator.evaluate(request(1.0, 0)).unwrap();

        assert_eq!(evaluated.scene.spheres[0].material_index, 0);
        assert_eq!(evaluated.counters.loop_counts["hits"], 0);
        assert_eq!(evaluated.counters.session_counts["hits"], 0);
    }

    #[test]
    fn disabled_physics_ignores_valid_actions_and_exposes_zero_counters() {
        let mut source = action_scene(false, TeleportVelocity::Clear);
        source.physics.as_mut().unwrap().enabled = false;
        let authored = source.spheres[0].center;
        let mut evaluator = PhysicsSceneEvaluator::new(source).unwrap();
        let evaluated = evaluator.evaluate(request(1.0, 0)).unwrap();

        assert_eq!(evaluated.scene.spheres[0].center, authored);
        assert_eq!(evaluated.counters.loop_counts["teleports"], 0);
        assert_eq!(evaluated.counters.session_counts["teleports"], 0);
        assert!(evaluated.physics_events.events.is_empty());
    }

    #[test]
    fn event_reaction_demo_flashes_and_reports_loop_and_session_counters() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/014_physics_event_reactions.json");
        let source = toaster_scene::load_scene(path).unwrap();
        let authored_counts = (source.spheres.len(), source.triangles.len());
        let ball = source
            .rigid_bodies
            .iter()
            .find(|body| body.id.as_str() == "ball_red")
            .unwrap();
        let ObjectBinding::Sphere { index } = ball.binding else {
            panic!("ball_red must remain an analytic sphere");
        };
        let authored_material = source.spheres[index].material_index;
        let flash_material = source
            .event_reactions
            .iter()
            .find_map(|reaction| {
                reaction
                    .flash
                    .as_ref()
                    .filter(|flash| flash.target.as_str() == "ball_red")
                    .map(|flash| flash.material_index)
            })
            .unwrap();
        let mut evaluator = PhysicsSceneEvaluator::new(source).unwrap();
        let first_loop = evaluator.evaluate(request(0.5, 0)).unwrap();

        assert_eq!(
            (
                first_loop.scene.spheres.len(),
                first_loop.scene.triangles.len()
            ),
            authored_counts
        );
        assert_ne!(authored_material, flash_material);
        assert_eq!(
            first_loop.scene.spheres[index].material_index,
            flash_material
        );
        assert!(first_loop.counters.loop_counts["collision_starts"] > 0);
        assert!(first_loop.counters.loop_counts["gate_entries"] > 0);
        assert!(first_loop.counters.loop_counts["orb_entries"] > 0);
        assert_eq!(
            first_loop.counters.loop_counts,
            first_loop.counters.session_counts
        );

        evaluator.evaluate(request(2.0, 0)).unwrap();
        let second_loop = evaluator.evaluate(request(0.5, 1)).unwrap();
        assert_eq!(
            second_loop.counters.loop_counts,
            first_loop.counters.loop_counts
        );
        assert!(
            second_loop.counters.session_counts["collision_starts"]
                > first_loop.counters.session_counts["collision_starts"]
        );
    }

    #[test]
    fn reset_teleport_demo_executes_actions_and_replays_loop_counts() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/015_physics_reset_teleport.json");
        let source = toaster_scene::load_scene(path).unwrap();
        let authored_counts = (source.spheres.len(), source.triangles.len());
        let mut evaluator = PhysicsSceneEvaluator::new(source).unwrap();
        let first = evaluator.evaluate(request(4.9, 0)).unwrap();

        assert_eq!(
            (first.scene.spheres.len(), first.scene.triangles.len()),
            authored_counts
        );
        assert!(first.counters.loop_counts["fall_resets"] > 0);
        assert!(first.counters.loop_counts["hazard_resets"] > 0);
        assert!(first.counters.loop_counts["goal_teleports"] > 0);
        let second = evaluator.evaluate(request(4.9, 1)).unwrap();
        assert_eq!(second.counters.loop_counts, first.counters.loop_counts);
        assert!(
            second.counters.session_counts["fall_resets"]
                > first.counters.session_counts["fall_resets"]
        );
        assert!(
            second.counters.session_counts["goal_teleports"]
                > first.counters.session_counts["goal_teleports"]
        );
    }

    #[test]
    fn counter_overflow_is_reported() {
        let mut counters = [("hits".to_owned(), u64::MAX)].into();
        assert!(increment_counter(&mut counters, "hits").is_err());
    }

    #[test]
    fn bouncing_contact_emits_collision_exit() {
        let mut scene = base_scene();
        scene.rigid_bodies[0].restitution = 1.0;
        scene.rigid_bodies[1].restitution = 1.0;
        let mut evaluator = PhysicsSceneEvaluator::new(scene).unwrap();
        let evaluated = evaluator.evaluate(request(2.0, 0)).unwrap();

        assert!(evaluated.physics_events.events.iter().any(|event| matches!(
            event.kind,
            PhysicsEventKind::Collision {
                phase: CollisionPhase::Exited,
                ..
            }
        )));
    }

    #[test]
    fn preserves_start_and_exit_within_one_nominal_tick() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/013_physics_events_triggers.json");
        let scene = toaster_scene::load_scene(path).unwrap();
        let mut evaluator = PhysicsSceneEvaluator::new(scene).unwrap();
        let evaluated = evaluator.evaluate(request(0.5, 0)).unwrap();
        let transitions = evaluated
            .physics_events
            .events
            .iter()
            .filter_map(|event| match &event.kind {
                PhysicsEventKind::Collision {
                    phase,
                    object_a,
                    object_b,
                } if object_a.as_str() == "ball_red" && object_b.as_str() == "sweeper" => {
                    Some((event.fixed_tick, *phase))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(transitions.windows(2).any(|events| {
            events[0] == (4, CollisionPhase::Started) && events[1] == (4, CollisionPhase::Exited)
        }));
    }

    #[test]
    fn trigger_sensor_emits_enter_and_exit_without_changing_motion() {
        let scene = trigger_scene();
        let mut with_trigger = PhysicsSceneEvaluator::new(scene.clone()).unwrap();
        let evaluated = with_trigger.evaluate(request(2.0, 0)).unwrap();
        let phases = evaluated
            .physics_events
            .events
            .iter()
            .filter_map(|event| match &event.kind {
                PhysicsEventKind::Trigger {
                    phase,
                    trigger,
                    object,
                } if trigger.as_str() == "zone" && object.as_str() == "ball" => Some(*phase),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(phases, vec![TriggerPhase::Entered, TriggerPhase::Exited]);
        let mut without_trigger_scene = scene;
        without_trigger_scene.triggers.clear();
        let mut without_trigger = PhysicsSceneEvaluator::new(without_trigger_scene).unwrap();
        let expected = without_trigger.evaluate(request(2.0, 0)).unwrap();
        assert_eq!(
            evaluated.scene.spheres[0].center,
            expected.scene.spheres[0].center
        );
    }

    #[test]
    fn kinematic_bodies_enter_fixed_trigger_sensors() {
        let mut scene = kinematic_scene();
        scene.triggers.push(TriggerDeclaration {
            id: PhysicsEntityId::new("upper_zone").unwrap(),
            center: Vec3::new(0.0, 0.8, 0.0),
            collider: ColliderShape::Cuboid {
                half_extents: Vec3::new(5.0, 0.1, 5.0),
            },
        });
        let mut evaluator = PhysicsSceneEvaluator::new(scene).unwrap();
        let evaluated = evaluator.evaluate(request(1.0, 0)).unwrap();

        assert!(evaluated.physics_events.events.iter().any(|event| matches!(
            &event.kind,
            PhysicsEventKind::Trigger {
                phase: TriggerPhase::Entered,
                trigger,
                object,
            } if trigger.as_str() == "upper_zone" && object.as_str() == "platform"
        )));
    }

    #[test]
    fn fixed_bodies_and_triggers_do_not_emit_sensor_events() {
        let mut scene = base_scene();
        scene.physics.as_mut().unwrap().gravity = Vec3::ZERO;
        scene.spheres[0].center = Vec3::new(20.0, 20.0, 20.0);
        scene.rigid_bodies[0].initial_transform.translation = scene.spheres[0].center;
        scene.triggers.push(TriggerDeclaration {
            id: PhysicsEntityId::new("floor_zone").unwrap(),
            center: Vec3::new(0.0, -0.1, 0.0),
            collider: ColliderShape::Cuboid {
                half_extents: Vec3::new(5.0, 0.2, 5.0),
            },
        });
        scene.triggers.push(TriggerDeclaration {
            id: PhysicsEntityId::new("overlapping_zone").unwrap(),
            center: Vec3::new(0.0, -0.1, 0.0),
            collider: ColliderShape::Sphere { radius: 1.0 },
        });
        let mut evaluator = PhysicsSceneEvaluator::new(scene).unwrap();
        let evaluated = evaluator.evaluate(request(0.25, 0)).unwrap();

        assert!(!evaluated
            .physics_events
            .events
            .iter()
            .any(|event| matches!(event.kind, PhysicsEventKind::Trigger { .. })));
    }

    #[test]
    fn frame_zero_is_the_exact_authored_geometry() {
        let source = base_scene();
        let mut evaluator = PhysicsSceneEvaluator::new(source.clone()).unwrap();
        let evaluated = evaluator.evaluate(request(0.0, 0)).unwrap();

        assert_eq!(evaluated.scene.spheres[0].center, source.spheres[0].center);
        assert_eq!(
            evaluated.scene.triangles[0].vertices,
            source.triangles[0].vertices
        );
        assert_eq!(
            evaluated.scene.triangles[0].material_index,
            source.triangles[0].material_index
        );
        assert_eq!(
            evaluated.scene.triangles[0].group,
            source.triangles[0].group
        );
    }

    #[test]
    fn reset_action_restores_authored_pose_velocity_and_clears_flash() {
        let mut scene = action_scene(true, TeleportVelocity::Clear);
        scene.materials.push(Material::Diffuse { albedo: Vec3::X });
        scene.event_reactions[0].flash = Some(MaterialFlashDeclaration {
            target: PhysicsEntityId::new("ball").unwrap(),
            material_index: 1,
            duration_seconds: 1.0,
        });
        let authored = scene.rigid_bodies[0].initial_transform;
        assert_eq!(authored.rotation, Quat::IDENTITY);
        let authored_scene = scene.clone();
        let mut evaluator = PhysicsSceneEvaluator::new(scene).unwrap();

        let reset = evaluator.evaluate(request(0.2, 0)).unwrap();
        assert!(reset.scene.spheres[0]
            .center
            .abs_diff_eq(authored.translation, 1.0e-5));
        assert_eq!(reset.scene.spheres[0].material_index, 0);
        assert_eq!(reset.counters.loop_counts["resets"], 1);
        assert!(reset.physics_events.events.iter().any(|event| matches!(
            event.kind,
            PhysicsEventKind::Trigger {
                phase: TriggerPhase::Entered,
                ..
            }
        )));
        assert!(!reset.physics_events.events.iter().any(|event| matches!(
            event.kind,
            PhysicsEventKind::Trigger {
                phase: TriggerPhase::Exited,
                ..
            }
        )));

        let moved = evaluator.evaluate(request(0.3, 0)).unwrap();
        assert!(moved.scene.spheres[0].center.x > authored.translation.x);
        assert_eq!(
            evaluator.source_scene().spheres[0].center,
            authored_scene.spheres[0].center
        );
    }

    #[test]
    fn teleport_clear_applies_between_crossed_ticks_and_exits_on_next_tick() {
        let scene = action_scene(false, TeleportVelocity::Clear);
        let source = scene.clone();
        let mut evaluator = PhysicsSceneEvaluator::new(scene).unwrap();

        let teleported = evaluator.evaluate(request(0.4, 0)).unwrap();
        assert!(teleported.scene.spheres[0]
            .center
            .abs_diff_eq(Vec3::new(3.0, 2.0, 0.0), 1.0e-4));
        assert_eq!(teleported.counters.loop_counts["teleports"], 1);
        let phases = teleported
            .physics_events
            .events
            .iter()
            .filter_map(|event| match event.kind {
                PhysicsEventKind::Trigger { phase, .. } => Some((event.fixed_tick, phase)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(phases[0], (2, TriggerPhase::Entered));
        assert!(phases.contains(&(3, TriggerPhase::Exited)));
        assert_eq!(source.spheres[0].center, Vec3::new(-1.0, 1.0, 0.0));
    }

    #[test]
    fn teleport_preserve_retains_linear_velocity() {
        let mut clear =
            PhysicsSceneEvaluator::new(action_scene(false, TeleportVelocity::Clear)).unwrap();
        let mut preserve =
            PhysicsSceneEvaluator::new(action_scene(false, TeleportVelocity::Preserve)).unwrap();

        let cleared = clear.evaluate(request(0.3, 0)).unwrap();
        let preserved = preserve.evaluate(request(0.3, 0)).unwrap();
        assert!((cleared.scene.spheres[0].center.x - 3.0).abs() < 1.0e-4);
        assert!(preserved.scene.spheres[0].center.x > cleared.scene.spheres[0].center.x + 0.1);
    }

    #[test]
    fn later_reaction_declaration_wins_for_the_same_action_target() {
        let mut scene = action_scene(false, TeleportVelocity::Clear);
        scene.spawn_points.push(SpawnPointDeclaration {
            name: "last_destination".into(),
            transform: RigidTransform {
                translation: Vec3::new(-3.0, 4.0, 1.0),
                rotation: Quat::IDENTITY,
            },
        });
        let mut later = scene.event_reactions[0].clone();
        later.teleport.as_mut().unwrap().spawn_point_index = 1;
        scene.event_reactions.push(later);
        let mut evaluator = PhysicsSceneEvaluator::new(scene).unwrap();

        let evaluated = evaluator.evaluate(request(0.2, 0)).unwrap();
        assert!(evaluated.scene.spheres[0]
            .center
            .abs_diff_eq(Vec3::new(-3.0, 4.0, 1.0), 1.0e-5));
        assert_eq!(evaluated.counters.loop_counts["teleports"], 2);
    }

    #[test]
    fn action_replay_matches_incremental_and_deduplicates_session_counts() {
        let scene = action_scene(false, TeleportVelocity::Clear);
        let mut incremental = PhysicsSceneEvaluator::new(scene.clone()).unwrap();
        incremental.evaluate(request(0.1, 0)).unwrap();
        let forward = incremental.evaluate(request(0.3, 0)).unwrap();
        incremental.evaluate(request(0.4, 0)).unwrap();
        let replayed = incremental.evaluate(request(0.3, 0)).unwrap();
        let mut fresh = PhysicsSceneEvaluator::new(scene).unwrap();
        let expected = fresh.evaluate(request(0.3, 0)).unwrap();

        assert_eq!(
            forward.scene.spheres[0].center,
            expected.scene.spheres[0].center
        );
        assert_eq!(
            replayed.scene.spheres[0].center,
            expected.scene.spheres[0].center
        );
        assert_eq!(forward.counters.loop_counts, expected.counters.loop_counts);
        assert_eq!(
            replayed.counters.session_counts,
            forward.counters.session_counts
        );

        let next_loop = incremental.evaluate(request(0.3, 1)).unwrap();
        assert_eq!(
            next_loop.counters.loop_counts,
            expected.counters.loop_counts
        );
        assert!(
            next_loop.counters.session_counts["teleports"]
                > forward.counters.session_counts["teleports"]
        );
    }

    #[test]
    fn kinematic_platform_follows_animation_and_pushes_dynamic_sphere() {
        let source = kinematic_scene();
        let authored_triangle_count = source.triangles.len();
        let authored_material = source.triangles[0].material_index;
        let mut evaluator = PhysicsSceneEvaluator::new(source.clone()).unwrap();
        let frame_zero = evaluator.evaluate(request(0.0, 0)).unwrap();
        let moved = evaluator.evaluate(request(0.5, 0)).unwrap();

        assert_eq!(frame_zero.scene.triangles[0].vertices[0].y, 0.0);
        assert!((moved.scene.triangles[0].vertices[0].y - 0.5).abs() < 1.0e-4);
        assert!(moved.scene.spheres[0].center.y > 0.85);
        assert_eq!(moved.scene.triangles.len(), authored_triangle_count);
        assert_eq!(moved.scene.triangles[0].material_index, authored_material);
        assert_eq!(moved.scene.triangles[0].group.as_deref(), Some("platform"));
        assert!(moved.changes.triangles);
        assert!(moved.changes.spheres);
        assert!(!moved.changes.emissive_geometry);
        assert_eq!(
            evaluator.source_scene().triangles[0].vertices,
            source.triangles[0].vertices
        );
    }

    #[test]
    fn kinematic_motion_replays_across_loops_and_backward_seeks() {
        let scene = kinematic_scene();
        let mut evaluator = PhysicsSceneEvaluator::new(scene.clone()).unwrap();
        let first = evaluator.evaluate(request(0.5, 0)).unwrap();
        evaluator.evaluate(request(0.9, 0)).unwrap();
        let replayed = evaluator.evaluate(request(0.5, 0)).unwrap();
        let next_loop = evaluator.evaluate(request(0.5, 1)).unwrap();
        let mut fresh = PhysicsSceneEvaluator::new(scene).unwrap();
        let expected = fresh.evaluate(request(0.5, 0)).unwrap();

        for candidate in [&first, &replayed, &next_loop] {
            assert_eq!(
                candidate.scene.spheres[0].center,
                expected.scene.spheres[0].center
            );
            assert_eq!(
                candidate.scene.triangles[0].vertices,
                expected.scene.triangles[0].vertices
            );
        }
    }

    #[test]
    fn moving_emissive_kinematic_geometry_invalidates_lights() {
        let mut scene = kinematic_scene();
        scene.materials[0] = Material::Emissive {
            color: Vec3::ONE,
            strength: 2.0,
        };
        let mut evaluator = PhysicsSceneEvaluator::new(scene).unwrap();
        evaluator.evaluate(request(0.0, 0)).unwrap();
        let moved = evaluator.evaluate(request(0.25, 0)).unwrap();

        assert!(moved.changes.emissive_geometry);
    }

    #[test]
    fn fixed_ticks_use_latest_completed_timestep() {
        let settings = base_scene().physics.unwrap();
        let tick_zero = PhysicsSceneEvaluator::physics_request(settings, request(0.0, 0)).unwrap();
        let tick_one =
            PhysicsSceneEvaluator::physics_request(settings, request(1.0 / 60.0, 0)).unwrap();
        let frame_at_24_fps =
            PhysicsSceneEvaluator::physics_request(settings, request(1.0 / 24.0, 0)).unwrap();

        assert_eq!(tick_zero.fixed_tick, 0);
        assert_eq!(tick_one.fixed_tick, 1);
        assert_eq!(frame_at_24_fps.fixed_tick, 2);
    }

    #[test]
    fn dynamic_cuboid_falls_rotates_and_preserves_render_identity() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/011_physics_rigid_bodies.json");
        let source = toaster_scene::load_scene(path).unwrap();
        let crate_body = source
            .rigid_bodies
            .iter()
            .find(|body| body.group.as_deref() == Some("crate_0"))
            .unwrap();
        let (start, count, pivot) = match crate_body.binding {
            ObjectBinding::Triangles {
                start,
                count,
                pivot,
            } => (start, count, pivot),
            ObjectBinding::Sphere { .. } => panic!("crate must bind generated triangles"),
        };
        let original_edge =
            (source.triangles[start].vertices[1] - source.triangles[start].vertices[0]).normalize();
        let material = source.triangles[start].material_index;
        let mut evaluator = PhysicsSceneEvaluator::new(source.clone()).unwrap();
        let evaluated = evaluator.evaluate(request(0.5, 0)).unwrap();
        let moved_edge = (evaluated.scene.triangles[start].vertices[1]
            - evaluated.scene.triangles[start].vertices[0])
            .normalize();
        let average_vertex = evaluated.scene.triangles[start..start + count]
            .iter()
            .flat_map(|triangle| triangle.vertices)
            .fold(Vec3::ZERO, |sum, vertex| sum + vertex)
            / (count * 3) as f32;

        assert!(average_vertex.y < pivot.y);
        assert!(original_edge.distance(moved_edge) > 0.01);
        assert!(evaluated.scene.triangles[start..start + count]
            .iter()
            .all(|triangle| triangle.material_index == material
                && triangle.group.as_deref() == Some("crate_0")));

        let static_floor = source
            .rigid_bodies
            .iter()
            .find(|body| body.group.as_deref() == Some("room_floor"))
            .unwrap();
        let (floor_start, floor_count) = match static_floor.binding {
            ObjectBinding::Triangles { start, count, .. } => (start, count),
            ObjectBinding::Sphere { .. } => panic!("floor must bind generated triangles"),
        };
        assert!(
            evaluated.scene.triangles[floor_start..floor_start + floor_count]
                .iter()
                .zip(&source.triangles[floor_start..floor_start + floor_count])
                .all(
                    |(evaluated, authored)| evaluated.vertices == authored.vertices
                        && evaluated.material_index == authored.material_index
                        && evaluated.group == authored.group
                )
        );
    }

    #[test]
    fn gltf_proxy_ranges_move_rigidly_and_static_mesh_stays_authored() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/016_physics_gltf_proxies.json");
        let source = toaster_scene::load_scene(path).unwrap();
        let dynamic = source
            .rigid_bodies
            .iter()
            .find(|body| body.id.as_str() == "visual_crate")
            .unwrap();
        let static_mesh = source
            .rigid_bodies
            .iter()
            .find(|body| body.id.as_str() == "terrain_pedestal")
            .unwrap();
        let sphere_proxy = source
            .rigid_bodies
            .iter()
            .find(|body| body.id.as_str() == "faceted_orb")
            .unwrap();
        let kinematic = source
            .rigid_bodies
            .iter()
            .find(|body| body.id.as_str() == "mesh_pusher")
            .unwrap();
        let (dynamic_start, dynamic_count) = match dynamic.binding {
            ObjectBinding::Triangles { start, count, .. } => (start, count),
            ObjectBinding::Sphere { .. } => panic!("mesh must bind triangles"),
        };
        let (static_start, static_count) = match static_mesh.binding {
            ObjectBinding::Triangles { start, count, .. } => (start, count),
            ObjectBinding::Sphere { .. } => panic!("mesh must bind triangles"),
        };
        let sphere_proxy_start = match sphere_proxy.binding {
            ObjectBinding::Triangles { start, .. } => start,
            ObjectBinding::Sphere { .. } => panic!("mesh must bind triangles"),
        };
        let kinematic_start = match kinematic.binding {
            ObjectBinding::Triangles { start, .. } => start,
            ObjectBinding::Sphere { .. } => panic!("mesh must bind triangles"),
        };
        let authored_materials = source.triangles[dynamic_start..dynamic_start + dynamic_count]
            .iter()
            .map(|triangle| triangle.material_index)
            .collect::<Vec<_>>();

        let mut evaluator = PhysicsSceneEvaluator::new(source.clone()).unwrap();
        let evaluated = evaluator.evaluate(request(0.75, 0)).unwrap();

        assert!(evaluated.changes.triangles);
        assert_eq!(evaluated.scene.triangles.len(), source.triangles.len());
        assert_ne!(
            evaluated.scene.triangles[dynamic_start].vertices,
            source.triangles[dynamic_start].vertices
        );
        assert_ne!(
            evaluated.scene.triangles[sphere_proxy_start].vertices,
            source.triangles[sphere_proxy_start].vertices
        );
        assert_ne!(
            evaluated.scene.triangles[kinematic_start].vertices,
            source.triangles[kinematic_start].vertices
        );
        assert_eq!(
            evaluated.scene.triangles[dynamic_start..dynamic_start + dynamic_count]
                .iter()
                .map(|triangle| triangle.material_index)
                .collect::<Vec<_>>(),
            authored_materials
        );
        assert!(
            evaluated.scene.triangles[static_start..static_start + static_count]
                .iter()
                .zip(&source.triangles[static_start..static_start + static_count])
                .all(|(evaluated, authored)| evaluated.vertices == authored.vertices)
        );
        assert!(evaluator
            .source_scene()
            .triangles
            .iter()
            .zip(&source.triangles)
            .all(|(actual, authored)| actual.vertices == authored.vertices
                && actual.material_index == authored.material_index
                && actual.group == authored.group));
    }

    #[test]
    fn restitution_changes_post_collision_motion() {
        let mut inelastic = base_scene();
        let mut elastic = inelastic.clone();
        elastic.rigid_bodies[0].restitution = 1.0;
        elastic.rigid_bodies[1].restitution = 1.0;
        inelastic.rigid_bodies[0].restitution = 0.0;
        inelastic.rigid_bodies[1].restitution = 0.0;

        let mut inelastic_eval = PhysicsSceneEvaluator::new(inelastic).unwrap();
        let mut elastic_eval = PhysicsSceneEvaluator::new(elastic).unwrap();
        let inelastic_y = inelastic_eval
            .evaluate(request(1.0, 0))
            .unwrap()
            .scene
            .spheres[0]
            .center
            .y;
        let elastic_y = elastic_eval
            .evaluate(request(1.0, 0))
            .unwrap()
            .scene
            .spheres[0]
            .center
            .y;

        assert!(elastic_y > inelastic_y + 0.25);
    }

    #[test]
    fn replay_and_incremental_evaluation_match() {
        let scene = base_scene();
        let mut incremental = PhysicsSceneEvaluator::new(scene.clone()).unwrap();
        let first = incremental.evaluate(request(0.75, 0)).unwrap();
        incremental.evaluate(request(1.5, 0)).unwrap();
        let replayed = incremental.evaluate(request(0.75, 0)).unwrap();
        let mut fresh = PhysicsSceneEvaluator::new(scene).unwrap();
        let expected = fresh.evaluate(request(0.75, 0)).unwrap();

        assert_eq!(
            first.scene.spheres[0].center,
            expected.scene.spheres[0].center
        );
        assert_eq!(
            replayed.scene.spheres[0].center,
            expected.scene.spheres[0].center
        );
    }

    #[test]
    fn loop_cycle_resets_world_before_replay() {
        let mut evaluator = PhysicsSceneEvaluator::new(base_scene()).unwrap();
        let first_cycle = evaluator.evaluate(request(1.5, 0)).unwrap();
        evaluator.evaluate(request(2.0, 0)).unwrap();
        let second_cycle = evaluator.evaluate(request(1.5, 1)).unwrap();
        assert_eq!(
            first_cycle.scene.spheres[0].center,
            second_cycle.scene.spheres[0].center
        );
        assert!(second_cycle.physics_events.reset);
        assert!(!first_cycle.physics_events.events.is_empty());
        assert_eq!(
            first_cycle.physics_events.events.len(),
            second_cycle.physics_events.events.len()
        );
        assert!(first_cycle
            .physics_events
            .events
            .iter()
            .zip(&second_cycle.physics_events.events)
            .all(|(first, second)| first.fixed_tick == second.fixed_tick
                && first.time_seconds == second.time_seconds
                && first.kind == second.kind));
    }

    #[test]
    fn disabled_physics_constructs_no_backend_and_leaves_bodies_ordinary() {
        let mut scene = base_scene();
        scene.physics.as_mut().unwrap().enabled = false;
        let authored_center = scene.spheres[0].center;
        let mut evaluator = PhysicsSceneEvaluator::new(scene).unwrap();

        assert!(evaluator.backend.is_none());
        assert!(!evaluator.is_time_varying());
        let evaluated = evaluator.evaluate(request(2.0, 4)).unwrap();
        assert_eq!(evaluated.scene.spheres[0].center, authored_center);
        assert_eq!(evaluated.physics_events, PhysicsEventBatch::default());
    }

    #[test]
    fn animation_only_evaluation_returns_an_empty_event_batch() {
        let mut scene = base_scene();
        scene.physics = None;
        scene.rigid_bodies.clear();
        let mut evaluator = toaster_scene::AnimationEvaluator::new(scene);

        let evaluated = evaluator.evaluate(request(0.5, 0)).unwrap();
        assert_eq!(evaluated.physics_events, PhysicsEventBatch::default());
    }

    struct FakeBackend {
        calls: Rc<RefCell<Vec<PhysicsEvaluationRequest>>>,
        resets: Rc<RefCell<u32>>,
    }

    struct EventOnlyBackend;

    impl PhysicsBackend for EventOnlyBackend {
        fn reset(&mut self) -> Result<()> {
            Ok(())
        }

        fn evaluate(&mut self, request: PhysicsEvaluationRequest) -> Result<EvaluatedPhysicsState> {
            Ok(EvaluatedPhysicsState {
                updates: Vec::new(),
                changes: SceneChanges::default(),
                physics_events: PhysicsEventBatch {
                    reset: false,
                    events: vec![PhysicsEvent {
                        loop_cycle: request.loop_cycle,
                        fixed_tick: request.fixed_tick,
                        time_seconds: request.scene_time_seconds,
                        kind: PhysicsEventKind::Trigger {
                            phase: TriggerPhase::Entered,
                            trigger: PhysicsEntityId::new("zone").unwrap(),
                            object: PhysicsEntityId::new("ball").unwrap(),
                        },
                    }],
                },
            })
        }

        fn apply_actions(&mut self, _actions: &[PhysicsBodyAction]) -> Result<()> {
            Ok(())
        }
    }

    impl PhysicsBackend for FakeBackend {
        fn reset(&mut self) -> Result<()> {
            *self.resets.borrow_mut() += 1;
            Ok(())
        }

        fn evaluate(&mut self, request: PhysicsEvaluationRequest) -> Result<EvaluatedPhysicsState> {
            self.calls.borrow_mut().push(request);
            Ok(EvaluatedPhysicsState {
                updates: vec![PhysicsSceneUpdate::RigidTransform {
                    binding: ObjectBinding::Sphere { index: 0 },
                    transform: RigidTransform {
                        translation: Vec3::new(1.0, 2.0, 3.0),
                        rotation: glam::Quat::IDENTITY,
                    },
                }],
                changes: SceneChanges {
                    spheres: true,
                    ..SceneChanges::default()
                },
                physics_events: PhysicsEventBatch {
                    reset: false,
                    events: vec![PhysicsEvent {
                        loop_cycle: request.loop_cycle,
                        fixed_tick: request.fixed_tick,
                        time_seconds: request.scene_time_seconds,
                        kind: PhysicsEventKind::Collision {
                            phase: CollisionPhase::Started,
                            object_a: PhysicsEntityId::new("ball").unwrap(),
                            object_b: PhysicsEntityId::new("floor").unwrap(),
                        },
                    }],
                },
            })
        }

        fn apply_actions(&mut self, _actions: &[PhysicsBodyAction]) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn neutral_backend_updates_scene_without_mutating_source() {
        let source = base_scene();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let resets = Rc::new(RefCell::new(0));
        let backend = FakeBackend {
            calls: calls.clone(),
            resets: resets.clone(),
        };
        let mut evaluator = PhysicsSceneEvaluator::with_backend(source, Box::new(backend)).unwrap();
        let evaluated = evaluator.evaluate(request(0.5, 2)).unwrap();
        evaluator.evaluate(request(0.25, 2)).unwrap();
        evaluator.evaluate(request(0.25, 3)).unwrap();

        assert_eq!(evaluated.scene.spheres[0].center, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(evaluator.source_scene().spheres[0].center.y, 3.0);
        assert_eq!(calls.borrow()[0].loop_cycle, 2);
        assert_eq!(calls.borrow()[0].fixed_tick, 30);
        assert_eq!(*resets.borrow(), 2);
        assert_eq!(evaluated.physics_events.events.len(), 1);
    }

    #[test]
    fn event_only_results_do_not_invalidate_renderer_data() {
        let mut evaluator =
            PhysicsSceneEvaluator::with_backend(base_scene(), Box::new(EventOnlyBackend)).unwrap();
        evaluator.evaluate(request(0.0, 0)).unwrap();
        let evaluated = evaluator.evaluate(request(1.0 / 60.0, 0)).unwrap();

        assert_eq!(evaluated.changes, SceneChanges::default());
        assert_eq!(evaluated.physics_events.events.len(), 1);
    }
}
