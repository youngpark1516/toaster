//! Rapier implementation of renderer-neutral rigid-body evaluation.

use crate::{EvaluatedPhysicsState, PhysicsBackend, PhysicsEvaluationRequest, PhysicsSceneUpdate};
use anyhow::{bail, Result};
use glam::{Quat, Vec3};
use rapier3d::prelude::*;
use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap},
    sync::mpsc::{self, Receiver},
};
use toaster_scene::{
    ColliderShape, CollisionPhase, GroupRigidTransformEvaluator, Material, ObjectBinding,
    PhysicsEntityId, PhysicsEvent, PhysicsEventBatch, PhysicsEventKind, RigidBodyDeclaration,
    RigidBodyKind, RigidTransform, Scene, SceneChanges, TriggerDeclaration, TriggerPhase,
};

#[derive(Clone)]
struct BoundBody {
    declaration: RigidBodyDeclaration,
    initial_transform: RigidTransform,
    kinematic_motion: Option<GroupRigidTransformEvaluator>,
    emissive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ColliderEntity {
    Object {
        id: PhysicsEntityId,
        body: RigidBodyKind,
    },
    Trigger {
        id: PhysicsEntityId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CollisionPair {
    object_a: PhysicsEntityId,
    object_b: PhysicsEntityId,
}

impl CollisionPair {
    fn new(first: PhysicsEntityId, second: PhysicsEntityId) -> Self {
        if first <= second {
            Self {
                object_a: first,
                object_b: second,
            }
        } else {
            Self {
                object_a: second,
                object_b: first,
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TriggerPair {
    trigger: PhysicsEntityId,
    object: PhysicsEntityId,
}

struct RapierWorld {
    pipeline: PhysicsPipeline,
    island_manager: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    handles: Vec<Option<RigidBodyHandle>>,
    collider_entities: HashMap<ColliderHandle, ColliderEntity>,
    collision_events: Receiver<CollisionEvent>,
    event_handler: ChannelEventCollector,
}

/// Rigid-body backend whose Rapier types never cross this crate boundary.
pub struct RapierRigidBodyBackend {
    gravity: Vec3,
    timestep: f32,
    substeps: u32,
    bound_bodies: Vec<BoundBody>,
    triggers: Vec<TriggerDeclaration>,
    world: RapierWorld,
    active_collisions: BTreeSet<CollisionPair>,
    active_triggers: BTreeSet<TriggerPair>,
    current_tick: u64,
    current_loop_cycle: Option<u64>,
}

impl RapierRigidBodyBackend {
    /// Constructs a rigid-body backend from validated renderer-neutral declarations.
    pub fn new(scene: &Scene) -> Result<Self> {
        let settings = scene
            .physics
            .ok_or_else(|| anyhow::anyhow!("Rapier backend requires physics settings"))?;
        if !settings.enabled {
            bail!("Rapier backend cannot be created for disabled physics");
        }
        let bound_bodies = scene
            .rigid_bodies
            .iter()
            .map(|declaration| {
                let authored_origin = initial_translation(scene, &declaration.binding)?;
                let kinematic_motion = if declaration.body == RigidBodyKind::Kinematic {
                    let group = declaration
                        .group
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("kinematic body has no animation group"))?;
                    Some(GroupRigidTransformEvaluator::new(
                        scene,
                        group,
                        authored_origin,
                    )?)
                } else {
                    None
                };
                let initial_transform = match &kinematic_motion {
                    Some(motion) => motion.evaluate(0.0)?,
                    None => RigidTransform {
                        translation: authored_origin,
                        rotation: Quat::IDENTITY,
                    },
                };
                Ok(BoundBody {
                    declaration: declaration.clone(),
                    initial_transform,
                    kinematic_motion,
                    emissive: binding_is_emissive(scene, &declaration.binding),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let triggers = scene.triggers.clone();
        let world = build_world(&bound_bodies, &triggers)?;
        Ok(Self {
            gravity: settings.gravity,
            timestep: settings.timestep,
            substeps: settings.substeps,
            bound_bodies,
            triggers,
            world,
            active_collisions: BTreeSet::new(),
            active_triggers: BTreeSet::new(),
            current_tick: 0,
            current_loop_cycle: None,
        })
    }

    fn step_nominal_tick(&mut self, loop_cycle: u64, events: &mut Vec<PhysicsEvent>) -> Result<()> {
        let gravity = rapier3d::math::Vector::new(self.gravity.x, self.gravity.y, self.gravity.z);
        let integration = IntegrationParameters {
            dt: self.timestep / self.substeps as f32,
            ..IntegrationParameters::default()
        };
        let fixed_tick = self.current_tick + 1;
        let active_at_start = self.active_collisions.clone();
        let mut transitioned_collisions = BTreeSet::new();
        for substep in 0..self.substeps {
            let substep_time =
                kinematic_substep_time(self.current_tick, substep, self.substeps, self.timestep);
            self.set_kinematic_targets(substep_time)?;
            self.world.pipeline.step(
                gravity,
                &integration,
                &mut self.world.island_manager,
                &mut self.world.broad_phase,
                &mut self.world.narrow_phase,
                &mut self.world.bodies,
                &mut self.world.colliders,
                &mut self.world.impulse_joints,
                &mut self.world.multibody_joints,
                &mut self.world.ccd_solver,
                &(),
                &self.world.event_handler,
            );
            self.drain_collision_events(
                loop_cycle,
                fixed_tick,
                substep_time,
                &mut transitioned_collisions,
                events,
            )?;
        }
        let tick_time = fixed_tick as f64 * self.timestep as f64;
        for pair in active_at_start.intersection(&self.active_collisions) {
            if transitioned_collisions.contains(pair) {
                continue;
            }
            events.push(PhysicsEvent {
                loop_cycle,
                fixed_tick,
                time_seconds: tick_time as f32,
                kind: PhysicsEventKind::Collision {
                    phase: CollisionPhase::Stayed,
                    object_a: pair.object_a.clone(),
                    object_b: pair.object_b.clone(),
                },
            });
        }
        self.current_tick += 1;
        Ok(())
    }

    fn drain_collision_events(
        &mut self,
        loop_cycle: u64,
        fixed_tick: u64,
        time_seconds: f32,
        transitioned_collisions: &mut BTreeSet<CollisionPair>,
        events: &mut Vec<PhysicsEvent>,
    ) -> Result<()> {
        let rapier_events = self.world.collision_events.try_iter().collect::<Vec<_>>();
        for event in rapier_events {
            let (first_handle, second_handle, started) = match event {
                CollisionEvent::Started(first, second, _) => (first, second, true),
                CollisionEvent::Stopped(first, second, _) => (first, second, false),
            };
            let first = self
                .world
                .collider_entities
                .get(&first_handle)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("collision event references unknown collider"))?;
            let second = self
                .world
                .collider_entities
                .get(&second_handle)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("collision event references unknown collider"))?;
            match (first, second) {
                (
                    ColliderEntity::Object { id: first, .. },
                    ColliderEntity::Object { id: second, .. },
                ) => {
                    let pair = CollisionPair::new(first, second);
                    let changed = if started {
                        self.active_collisions.insert(pair.clone())
                    } else {
                        self.active_collisions.remove(&pair)
                    };
                    if changed {
                        transitioned_collisions.insert(pair.clone());
                        events.push(PhysicsEvent {
                            loop_cycle,
                            fixed_tick,
                            time_seconds,
                            kind: PhysicsEventKind::Collision {
                                phase: if started {
                                    CollisionPhase::Started
                                } else {
                                    CollisionPhase::Exited
                                },
                                object_a: pair.object_a,
                                object_b: pair.object_b,
                            },
                        });
                    }
                }
                (ColliderEntity::Trigger { id: trigger }, ColliderEntity::Object { id, body })
                | (ColliderEntity::Object { id, body }, ColliderEntity::Trigger { id: trigger }) => {
                    if body == RigidBodyKind::Static {
                        continue;
                    }
                    let pair = TriggerPair {
                        trigger,
                        object: id,
                    };
                    let changed = if started {
                        self.active_triggers.insert(pair.clone())
                    } else {
                        self.active_triggers.remove(&pair)
                    };
                    if changed {
                        events.push(PhysicsEvent {
                            loop_cycle,
                            fixed_tick,
                            time_seconds,
                            kind: PhysicsEventKind::Trigger {
                                phase: if started {
                                    TriggerPhase::Entered
                                } else {
                                    TriggerPhase::Exited
                                },
                                trigger: pair.trigger,
                                object: pair.object,
                            },
                        });
                    }
                }
                (ColliderEntity::Trigger { .. }, ColliderEntity::Trigger { .. }) => {}
            }
        }
        Ok(())
    }

    fn set_kinematic_targets(&mut self, time_seconds: f32) -> Result<()> {
        for (bound, handle) in self.bound_bodies.iter().zip(&self.world.handles) {
            let Some(motion) = &bound.kinematic_motion else {
                continue;
            };
            let handle = handle.ok_or_else(|| anyhow::anyhow!("kinematic body has no handle"))?;
            let body = self
                .world
                .bodies
                .get_mut(handle)
                .ok_or_else(|| anyhow::anyhow!("Rapier body handle is stale"))?;
            body.set_next_kinematic_position(rapier_pose(motion.evaluate(time_seconds)?));
        }
        Ok(())
    }

    fn evaluated_state(&self, physics_events: PhysicsEventBatch) -> Result<EvaluatedPhysicsState> {
        let mut updates = Vec::new();
        let mut changes = SceneChanges::default();
        for (bound, handle) in self.bound_bodies.iter().zip(&self.world.handles) {
            if !matches!(
                bound.declaration.body,
                RigidBodyKind::Dynamic | RigidBodyKind::Kinematic
            ) {
                continue;
            }
            let handle = handle.ok_or_else(|| anyhow::anyhow!("moving body has no handle"))?;
            let body = self
                .world
                .bodies
                .get(handle)
                .ok_or_else(|| anyhow::anyhow!("Rapier body handle is stale"))?;
            let translation = body.translation();
            let rotation = body.rotation();
            updates.push(PhysicsSceneUpdate::RigidTransform {
                binding: bound.declaration.binding.clone(),
                transform: RigidTransform {
                    translation: Vec3::new(translation.x, translation.y, translation.z),
                    rotation: Quat::from_xyzw(rotation.x, rotation.y, rotation.z, rotation.w),
                },
            });
            match bound.declaration.binding {
                ObjectBinding::Sphere { .. } => changes.spheres = true,
                ObjectBinding::Triangles { .. } => changes.triangles = true,
            }
            changes.emissive_geometry |= bound.emissive;
        }
        Ok(EvaluatedPhysicsState {
            updates,
            changes,
            physics_events,
        })
    }
}

fn kinematic_substep_time(current_tick: u64, substep: u32, substeps: u32, timestep: f32) -> f32 {
    ((current_tick as f64 + (substep + 1) as f64 / substeps as f64) * timestep as f64) as f32
}

impl PhysicsBackend for RapierRigidBodyBackend {
    fn reset(&mut self) -> Result<()> {
        self.world = build_world(&self.bound_bodies, &self.triggers)?;
        self.active_collisions.clear();
        self.active_triggers.clear();
        self.current_tick = 0;
        self.current_loop_cycle = None;
        Ok(())
    }

    fn evaluate(&mut self, request: PhysicsEvaluationRequest) -> Result<EvaluatedPhysicsState> {
        if !request.scene_time_seconds.is_finite() || request.scene_time_seconds < 0.0 {
            bail!("physics evaluation time must be finite and non-negative");
        }
        let mut reset = false;
        if self.current_loop_cycle.is_some_and(|cycle| {
            cycle != request.loop_cycle || request.fixed_tick < self.current_tick
        }) {
            self.reset()?;
            self.current_loop_cycle = Some(request.loop_cycle);
            reset = true;
        } else if self.current_loop_cycle.is_none() {
            self.current_loop_cycle = Some(request.loop_cycle);
        }
        let mut events = Vec::new();
        while self.current_tick < request.fixed_tick {
            self.step_nominal_tick(request.loop_cycle, &mut events)?;
        }
        events.sort_by(compare_physics_events);
        self.evaluated_state(PhysicsEventBatch { reset, events })
    }
}

fn build_world(bound_bodies: &[BoundBody], triggers: &[TriggerDeclaration]) -> Result<RapierWorld> {
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    let mut handles = Vec::with_capacity(bound_bodies.len());
    let mut collider_entities = HashMap::new();
    for bound in bound_bodies {
        let mut body_builder = match bound.declaration.body {
            RigidBodyKind::Static => RigidBodyBuilder::fixed(),
            RigidBodyKind::Dynamic => RigidBodyBuilder::dynamic()
                .linvel(rapier3d::math::Vector::new(
                    bound.declaration.initial_velocity.x,
                    bound.declaration.initial_velocity.y,
                    bound.declaration.initial_velocity.z,
                ))
                .angvel(rapier3d::math::Vector::new(
                    bound.declaration.initial_angular_velocity.x,
                    bound.declaration.initial_angular_velocity.y,
                    bound.declaration.initial_angular_velocity.z,
                )),
            RigidBodyKind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        };
        body_builder = body_builder.pose(rapier_pose(bound.initial_transform));
        let handle = bodies.insert(body_builder.build());
        let mut collider_builder = match bound.declaration.collider {
            ColliderShape::Sphere { radius } => ColliderBuilder::ball(radius),
            ColliderShape::Cuboid { half_extents } => {
                ColliderBuilder::cuboid(half_extents.x, half_extents.y, half_extents.z)
            }
        }
        .friction(bound.declaration.friction)
        .restitution(bound.declaration.restitution)
        .active_events(ActiveEvents::COLLISION_EVENTS);
        if let Some(mass) = bound.declaration.mass {
            collider_builder = collider_builder.mass(mass);
        }
        let collider_handle =
            colliders.insert_with_parent(collider_builder.build(), handle, &mut bodies);
        collider_entities.insert(
            collider_handle,
            ColliderEntity::Object {
                id: bound.declaration.id.clone(),
                body: bound.declaration.body,
            },
        );
        handles.push(Some(handle));
    }

    for trigger in triggers {
        let builder = match trigger.collider {
            ColliderShape::Sphere { radius } => ColliderBuilder::ball(radius),
            ColliderShape::Cuboid { half_extents } => {
                ColliderBuilder::cuboid(half_extents.x, half_extents.y, half_extents.z)
            }
        }
        .translation(rapier3d::math::Vector::new(
            trigger.center.x,
            trigger.center.y,
            trigger.center.z,
        ))
        .sensor(true)
        .active_events(ActiveEvents::COLLISION_EVENTS)
        .active_collision_types(
            ActiveCollisionTypes::default() | ActiveCollisionTypes::KINEMATIC_FIXED,
        );
        let handle = colliders.insert(builder.build());
        collider_entities.insert(
            handle,
            ColliderEntity::Trigger {
                id: trigger.id.clone(),
            },
        );
    }

    let (collision_sender, collision_events) = mpsc::channel();
    let (force_sender, _force_events) = mpsc::channel();
    let event_handler = ChannelEventCollector::new(collision_sender, force_sender);

    Ok(RapierWorld {
        pipeline: PhysicsPipeline::new(),
        island_manager: IslandManager::new(),
        broad_phase: DefaultBroadPhase::new(),
        narrow_phase: NarrowPhase::new(),
        bodies,
        colliders,
        impulse_joints: ImpulseJointSet::new(),
        multibody_joints: MultibodyJointSet::new(),
        ccd_solver: CCDSolver::new(),
        handles,
        collider_entities,
        collision_events,
        event_handler,
    })
}

fn compare_physics_events(first: &PhysicsEvent, second: &PhysicsEvent) -> Ordering {
    first
        .fixed_tick
        .cmp(&second.fixed_tick)
        .then_with(|| first.time_seconds.total_cmp(&second.time_seconds))
        .then_with(|| event_sort_key(&first.kind).cmp(&event_sort_key(&second.kind)))
}

fn event_sort_key(kind: &PhysicsEventKind) -> (u8, u8, &str, &str) {
    match kind {
        PhysicsEventKind::Collision {
            phase,
            object_a,
            object_b,
        } => (
            match phase {
                CollisionPhase::Started => 0,
                CollisionPhase::Stayed => 1,
                CollisionPhase::Exited => 2,
            },
            0,
            object_a.as_str(),
            object_b.as_str(),
        ),
        PhysicsEventKind::Trigger {
            phase,
            trigger,
            object,
        } => (
            match phase {
                TriggerPhase::Entered => 0,
                TriggerPhase::Exited => 2,
            },
            1,
            trigger.as_str(),
            object.as_str(),
        ),
    }
}

fn rapier_pose(transform: RigidTransform) -> Pose {
    Pose::from_parts(
        rapier3d::math::Vector::new(
            transform.translation.x,
            transform.translation.y,
            transform.translation.z,
        ),
        rapier3d::math::Rotation::from_xyzw(
            transform.rotation.x,
            transform.rotation.y,
            transform.rotation.z,
            transform.rotation.w,
        )
        .normalize(),
    )
}

fn initial_translation(scene: &Scene, binding: &ObjectBinding) -> Result<Vec3> {
    match *binding {
        ObjectBinding::Sphere { index } => scene
            .spheres
            .get(index)
            .map(|sphere| sphere.center)
            .ok_or_else(|| anyhow::anyhow!("sphere binding is out of range")),
        ObjectBinding::Triangles { pivot, .. } => Ok(pivot),
    }
}

fn binding_is_emissive(scene: &Scene, binding: &ObjectBinding) -> bool {
    let material_indices: Box<dyn Iterator<Item = usize> + '_> = match *binding {
        ObjectBinding::Sphere { index } => Box::new(
            scene
                .spheres
                .get(index)
                .into_iter()
                .map(|sphere| sphere.material_index),
        ),
        ObjectBinding::Triangles { start, count, .. } => Box::new(
            scene
                .triangles
                .get(start..start.saturating_add(count))
                .into_iter()
                .flatten()
                .map(|triangle| triangle.material_index),
        ),
    };
    material_indices.into_iter().any(|index| {
        matches!(
            scene.materials.get(index),
            Some(Material::Emissive { strength, .. }) if *strength > 0.0
        )
    })
}

#[cfg(test)]
mod tests {
    use super::kinematic_substep_time;

    #[test]
    fn samples_each_kinematic_substep_endpoint() {
        assert_eq!(kinematic_substep_time(0, 0, 4, 1.0), 0.25);
        assert_eq!(kinematic_substep_time(0, 3, 4, 1.0), 1.0);
        assert_eq!(kinematic_substep_time(2, 1, 2, 0.5), 1.5);
    }
}
