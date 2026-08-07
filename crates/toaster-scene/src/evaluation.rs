//! Renderer-neutral frame evaluation contracts shared by all renderers.

use crate::{AnimationTarget, Material, Scene};
use anyhow::Result;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// Categories of renderer data changed by one scene evaluation.
pub struct SceneChanges {
    /// Camera basis or position changed.
    pub camera: bool,
    /// At least one analytic sphere changed.
    pub spheres: bool,
    /// At least one triangle or its attributes changed.
    pub triangles: bool,
    /// At least one moved primitive uses an emissive material.
    pub emissive_geometry: bool,
}

impl SceneChanges {
    /// Returns change metadata for a complete first-frame upload.
    pub fn all() -> Self {
        Self {
            camera: true,
            spheres: true,
            triangles: true,
            emissive_geometry: true,
        }
    }

    /// Combines change categories produced by independent evaluators.
    pub fn union(self, other: Self) -> Self {
        Self {
            camera: self.camera || other.camera,
            spheres: self.spheres || other.spheres,
            triangles: self.triangles || other.triangles,
            emissive_geometry: self.emissive_geometry || other.emissive_geometry,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Scene-local time and explicit loop identity requested by a renderer.
pub struct EvaluationRequest {
    /// Wrapped scene time in seconds.
    pub time_seconds: f32,
    /// Zero-based loop cycle; always zero for schedules without wrapping.
    pub loop_cycle: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// CPU wall-clock diagnostics for renderer-neutral frame evaluation.
pub struct SceneEvaluationTimings {
    /// Time spent cloning the base scene and applying allowed animation tracks.
    pub animation_evaluation: Duration,
    /// Time spent resetting, replaying, or incrementally stepping a physics backend.
    pub physics_evaluation: Duration,
    /// Time spent applying backend-neutral updates to evaluated render geometry.
    pub geometry_update: Duration,
}

#[derive(Clone, Debug)]
/// Fully evaluated renderer-neutral scene plus upload change metadata.
pub struct EvaluatedScene {
    /// Ordinary sphere/triangle scene consumed by CPU and GPU renderers.
    pub scene: Scene,
    /// Data categories changed since the evaluator's preceding request.
    pub changes: SceneChanges,
    /// Optional-cost diagnostics gathered by the evaluator for this request.
    pub timings: SceneEvaluationTimings,
}

/// Stateful or stateless provider of evaluated renderer-neutral frames.
pub trait SceneEvaluator {
    /// Returns the immutable authored source scene.
    fn source_scene(&self) -> &Scene;

    /// Reports whether evaluation may produce different scene geometry or camera data over time.
    fn is_time_varying(&self) -> bool;

    /// Evaluates one reproducible scene-local frame.
    fn evaluate(&mut self, request: EvaluationRequest) -> Result<EvaluatedScene>;
}

#[derive(Clone, Debug)]
/// Stateless adapter for scenes driven only by built-in animation tracks.
pub struct AnimationEvaluator {
    source: Scene,
    evaluated_once: bool,
}

impl AnimationEvaluator {
    /// Takes ownership of an immutable source scene.
    pub fn new(source: Scene) -> Self {
        Self {
            source,
            evaluated_once: false,
        }
    }
}

impl SceneEvaluator for AnimationEvaluator {
    fn source_scene(&self) -> &Scene {
        &self.source
    }

    fn is_time_varying(&self) -> bool {
        !self.source.animation.tracks.is_empty()
    }

    fn evaluate(&mut self, request: EvaluationRequest) -> Result<EvaluatedScene> {
        let evaluation_start = Instant::now();
        let scene = self.source.evaluate_at(request.time_seconds)?;
        let timings = SceneEvaluationTimings {
            animation_evaluation: evaluation_start.elapsed(),
            ..SceneEvaluationTimings::default()
        };
        let changes = if self.evaluated_once {
            animation_changes(&self.source)
        } else {
            SceneChanges::all()
        };
        self.evaluated_once = true;
        Ok(EvaluatedScene {
            scene,
            changes,
            timings,
        })
    }
}

/// Computes conservative renderer changes caused by declared animation tracks.
pub fn animation_changes(scene: &Scene) -> SceneChanges {
    let mut changes = SceneChanges::default();
    for track in &scene.animation.tracks {
        match track.target() {
            AnimationTarget::Camera => changes.camera = true,
            AnimationTarget::Group { name } => {
                for sphere in &scene.spheres {
                    if sphere.group.as_deref() == Some(name) {
                        changes.spheres = true;
                        changes.emissive_geometry |= is_emissive(scene, sphere.material_index);
                    }
                }
                for triangle in &scene.triangles {
                    if triangle.group.as_deref() == Some(name) {
                        changes.triangles = true;
                        changes.emissive_geometry |= is_emissive(scene, triangle.material_index);
                    }
                }
            }
        }
    }
    changes
}

fn is_emissive(scene: &Scene, material_index: usize) -> bool {
    matches!(
        scene.materials.get(material_index),
        Some(Material::Emissive { strength, .. }) if *strength > 0.0
    )
}
