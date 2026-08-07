//! Renderer-neutral physics evaluation with replaceable backend implementations.

mod rapier_backend;

use anyhow::{bail, Context, Result};
use toaster_scene::{
    animation_changes, apply_rigid_transform, EvaluatedScene, EvaluationRequest, ObjectBinding,
    RigidTransform, Scene, SceneChanges, SceneEvaluator,
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
        let mut scene = self.source.evaluate_at(request.time_seconds)?;
        let mut changes = animation_changes(&self.source);
        if let Some(backend) = &mut self.backend {
            let settings = self
                .source
                .physics
                .context("active physics evaluator requires physics settings")?;
            let backend_request = Self::physics_request(settings, request)?;
            if self.last_physics_request.is_some_and(|previous| {
                previous.loop_cycle != backend_request.loop_cycle
                    || backend_request.fixed_tick < previous.fixed_tick
            }) {
                backend.reset()?;
            }
            let state = backend.evaluate(backend_request)?;
            self.last_physics_request = Some(backend_request);
            for update in state.updates {
                match update {
                    PhysicsSceneUpdate::RigidTransform { binding, transform } => {
                        apply_rigid_transform(&self.source, &mut scene, &binding, transform)?;
                    }
                }
            }
            changes = changes.union(state.changes);
        }
        if !self.evaluated_once {
            changes = SceneChanges::all();
        }
        self.evaluated_once = true;
        Ok(EvaluatedScene { scene, changes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use std::{cell::RefCell, rc::Rc};
    use toaster_scene::{
        Background, CameraSettings, ColliderShape, Material, PhysicsSettings, PhysicsType,
        RenderSettings, RigidBodyDeclaration, RigidBodyKind, Sphere, Triangle, TriangleAttributes,
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
        }
    }

    fn request(time_seconds: f32, loop_cycle: u64) -> EvaluationRequest {
        EvaluationRequest {
            time_seconds,
            loop_cycle,
        }
    }

    #[test]
    fn sphere_falls_and_collides_with_static_floor() {
        let mut evaluator = PhysicsSceneEvaluator::new(base_scene()).unwrap();
        let falling = evaluator.evaluate(request(0.5, 0)).unwrap();
        assert!(falling.scene.spheres[0].center.y < 3.0);
        let settled = evaluator.evaluate(request(3.0, 0)).unwrap();
        assert!(settled.scene.spheres[0].center.y >= 0.49);
        assert!(settled.scene.spheres[0].center.y < 0.6);
        assert_eq!(evaluator.source_scene().spheres[0].center.y, 3.0);
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
        let first_cycle = evaluator.evaluate(request(0.5, 0)).unwrap();
        evaluator.evaluate(request(2.0, 0)).unwrap();
        let second_cycle = evaluator.evaluate(request(0.5, 1)).unwrap();
        assert_eq!(
            first_cycle.scene.spheres[0].center,
            second_cycle.scene.spheres[0].center
        );
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
    }

    struct FakeBackend {
        calls: Rc<RefCell<Vec<PhysicsEvaluationRequest>>>,
        resets: Rc<RefCell<u32>>,
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
    }
}
