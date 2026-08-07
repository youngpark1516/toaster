//! Rapier implementation of renderer-neutral rigid-body evaluation.

use crate::{EvaluatedPhysicsState, PhysicsBackend, PhysicsEvaluationRequest, PhysicsSceneUpdate};
use anyhow::{bail, Result};
use glam::{Quat, Vec3};
use rapier3d::prelude::*;
use toaster_scene::{
    ColliderShape, GroupRigidTransformEvaluator, Material, ObjectBinding, RigidBodyDeclaration,
    RigidBodyKind, RigidTransform, Scene, SceneChanges,
};

#[derive(Clone)]
struct BoundBody {
    declaration: RigidBodyDeclaration,
    initial_transform: RigidTransform,
    kinematic_motion: Option<GroupRigidTransformEvaluator>,
    emissive: bool,
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
}

/// Rigid-body backend whose Rapier types never cross this crate boundary.
pub struct RapierRigidBodyBackend {
    gravity: Vec3,
    timestep: f32,
    substeps: u32,
    bound_bodies: Vec<BoundBody>,
    world: RapierWorld,
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
        let world = build_world(&bound_bodies)?;
        Ok(Self {
            gravity: settings.gravity,
            timestep: settings.timestep,
            substeps: settings.substeps,
            bound_bodies,
            world,
            current_tick: 0,
            current_loop_cycle: None,
        })
    }

    fn step_nominal_tick(&mut self) -> Result<()> {
        let gravity = rapier3d::math::Vector::new(self.gravity.x, self.gravity.y, self.gravity.z);
        let integration = IntegrationParameters {
            dt: self.timestep / self.substeps as f32,
            ..IntegrationParameters::default()
        };
        for substep in 0..self.substeps {
            self.set_kinematic_targets(kinematic_substep_time(
                self.current_tick,
                substep,
                self.substeps,
                self.timestep,
            ))?;
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
                &(),
            );
        }
        self.current_tick += 1;
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

    fn evaluated_state(&self) -> Result<EvaluatedPhysicsState> {
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
        Ok(EvaluatedPhysicsState { updates, changes })
    }
}

fn kinematic_substep_time(current_tick: u64, substep: u32, substeps: u32, timestep: f32) -> f32 {
    ((current_tick as f64 + (substep + 1) as f64 / substeps as f64) * timestep as f64) as f32
}

impl PhysicsBackend for RapierRigidBodyBackend {
    fn reset(&mut self) -> Result<()> {
        self.world = build_world(&self.bound_bodies)?;
        self.current_tick = 0;
        self.current_loop_cycle = None;
        Ok(())
    }

    fn evaluate(&mut self, request: PhysicsEvaluationRequest) -> Result<EvaluatedPhysicsState> {
        if !request.scene_time_seconds.is_finite() || request.scene_time_seconds < 0.0 {
            bail!("physics evaluation time must be finite and non-negative");
        }
        if self.current_loop_cycle != Some(request.loop_cycle)
            || request.fixed_tick < self.current_tick
        {
            self.reset()?;
            self.current_loop_cycle = Some(request.loop_cycle);
        } else if self.current_loop_cycle.is_none() {
            self.current_loop_cycle = Some(request.loop_cycle);
        }
        while self.current_tick < request.fixed_tick {
            self.step_nominal_tick()?;
        }
        self.evaluated_state()
    }
}

fn build_world(bound_bodies: &[BoundBody]) -> Result<RapierWorld> {
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    let mut handles = Vec::with_capacity(bound_bodies.len());
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
        .restitution(bound.declaration.restitution);
        if let Some(mass) = bound.declaration.mass {
            collider_builder = collider_builder.mass(mass);
        }
        colliders.insert_with_parent(collider_builder.build(), handle, &mut bodies);
        handles.push(Some(handle));
    }

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
    })
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
