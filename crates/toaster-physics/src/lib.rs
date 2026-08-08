//! Renderer-neutral physics evaluation with replaceable backend implementations.

mod rapier_backend;

use anyhow::{bail, Context, Result};
use std::time::Instant;
use toaster_scene::{
    animation_changes, apply_rigid_transform, EvaluatedScene, EvaluationRequest, ObjectBinding,
    PhysicsEventBatch, RigidTransform, Scene, SceneChanges, SceneEvaluationTimings, SceneEvaluator,
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
}

/// Composes built-in animation and an optional physics backend from an immutable scene.
pub struct PhysicsSceneEvaluator {
    source: Scene,
    backend: Option<Box<dyn PhysicsBackend>>,
    evaluated_once: bool,
    last_physics_request: Option<PhysicsEvaluationRequest>,
}

impl PhysicsSceneEvaluator {
    /// Builds the default backend selected by the scene's simulation domain.
    pub fn new(source: Scene) -> Result<Self> {
        let backend = create_backend(&source)?;
        Ok(Self {
            source,
            backend,
            evaluated_once: false,
            last_physics_request: None,
        })
    }

    /// Installs a caller-supplied backend, primarily for alternative solvers and tests.
    pub fn with_backend(source: Scene, backend: Box<dyn PhysicsBackend>) -> Result<Self> {
        if !source.physics.is_some_and(|settings| settings.enabled) {
            bail!("a custom physics backend requires physics.enabled to be true");
        }
        Ok(Self {
            source,
            backend: Some(backend),
            evaluated_once: false,
            last_physics_request: None,
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
            });
            if reset {
                backend.reset()?;
            }
            let mut state = backend.evaluate(backend_request)?;
            state.physics_events.reset |= reset;
            physics_evaluation = physics_start.elapsed();
            self.last_physics_request = Some(backend_request);
            if state.physics_events.reset {
                tracing::debug!(
                    loop_cycle = backend_request.loop_cycle,
                    fixed_tick = backend_request.fixed_tick,
                    "reset physics event timeline before replay"
                );
            }
            for event in &state.physics_events.events {
                tracing::debug!(
                    loop_cycle = event.loop_cycle,
                    fixed_tick = event.fixed_tick,
                    event_time_seconds = event.time_seconds as f64,
                    event = ?event.kind,
                    "physics event"
                );
            }
            physics_events = state.physics_events;
            let geometry_start = Instant::now();
            for update in state.updates {
                match update {
                    PhysicsSceneUpdate::RigidTransform { binding, transform } => {
                        apply_rigid_transform(&self.source, &mut scene, &binding, transform)?;
                    }
                }
            }
            geometry_update = geometry_start.elapsed();
            changes = changes.union(state.changes);
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use std::{cell::RefCell, rc::Rc};
    use toaster_scene::{
        Animation, AnimationTarget, AnimationTrack, Background, CameraSettings, ColliderShape,
        CollisionPhase, Interpolation, Material, PhysicsEntityId, PhysicsEvent, PhysicsEventKind,
        PhysicsSettings, PhysicsType, RenderSettings, RigidBodyDeclaration, RigidBodyKind, Sphere,
        TranslationKeyframe, Triangle, TriangleAttributes, TriggerDeclaration, TriggerPhase,
    };

    fn base_scene() -> Scene {
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
                    binding: floor_binding,
                    group: Some("floor".into()),
                },
            ],
            triggers: Vec::new(),
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
