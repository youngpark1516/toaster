//! Renderer-independent scene animation tracks and evaluation.

use crate::{ObjectBinding, RigidBodyKind, RigidTransform, Scene};
use anyhow::{bail, Result};
use glam::{Quat, Vec3};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, Deserialize)]
/// Ordered animation tracks evaluated from an immutable base scene.
pub struct Animation {
    /// Tracks declared by the scene, preserved in declaration order.
    #[serde(default)]
    pub tracks: Vec<AnimationTrack>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// A camera or named object group affected by a track.
pub enum AnimationTarget {
    /// Every sphere and triangle with the matching group name.
    Group {
        /// Nonempty group name.
        name: String,
    },
    /// The scene camera.
    Camera,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Interpolation applied between adjacent keyframes.
pub enum Interpolation {
    /// Blend continuously between keyframe values.
    Linear,
    /// Hold the left keyframe until the next keyframe time.
    Step,
}

#[derive(Clone, Copy, Debug, Deserialize)]
/// Translation offset at one animation time.
pub struct TranslationKeyframe {
    /// Nonnegative time in seconds.
    pub time: f32,
    /// World-space translation offset from the base pose.
    pub value: Vec3,
}

#[derive(Clone, Copy, Debug, Deserialize)]
/// Rotation angle at one animation time.
pub struct RotationKeyframe {
    /// Nonnegative time in seconds.
    pub time: f32,
    /// Rotation angle in degrees from the base pose.
    pub degrees: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// A translation or axis/pivot rotation track.
pub enum AnimationTrack {
    /// Translates a target by sampled offsets.
    Translation {
        /// Camera or object group to translate.
        target: AnimationTarget,
        /// Sampling rule between keyframes.
        interpolation: Interpolation,
        /// Strictly time-ordered offsets.
        keyframes: Vec<TranslationKeyframe>,
    },
    /// Rotates a target around a fixed world-space axis and pivot.
    Rotation {
        /// Camera or object group to rotate.
        target: AnimationTarget,
        /// Finite nonzero axis normalized during scene loading.
        axis: Vec3,
        /// World-space rotation pivot.
        pivot: Vec3,
        /// Sampling rule between keyframes.
        interpolation: Interpolation,
        /// Strictly time-ordered angles.
        keyframes: Vec<RotationKeyframe>,
    },
}

#[derive(Clone, Debug)]
/// Prevalidated animation tracks that produce one group's absolute rigid pose.
pub struct GroupRigidTransformEvaluator {
    authored_origin: Vec3,
    tracks: Vec<AnimationTrack>,
}

impl AnimationTrack {
    /// Returns the camera or group affected by this track.
    pub fn target(&self) -> &AnimationTarget {
        match self {
            Self::Translation { target, .. } | Self::Rotation { target, .. } => target,
        }
    }
}

impl Animation {
    /// Validates and normalizes every rotation axis in place.
    pub(crate) fn normalize_rotation_axes(&mut self) -> Result<()> {
        for track in &mut self.tracks {
            let AnimationTrack::Rotation { axis, .. } = track else {
                continue;
            };
            if !axis.is_finite() || axis.length_squared() <= f32::EPSILON {
                bail!("rotation axis must be finite and non-zero");
            }
            *axis = axis.normalize();
        }
        Ok(())
    }
}

impl Scene {
    /// Evaluates all tracks from this immutable base scene at `time_seconds`.
    ///
    /// Translation tracks compose before rotations; rotations retain declaration
    /// order. The base scene is cloned and remains unchanged.
    pub fn evaluate_at(&self, time_seconds: f32) -> Result<Self> {
        if !time_seconds.is_finite() || time_seconds < 0.0 {
            bail!("animation time must be finite and non-negative");
        }
        validate_animation(self)?;

        let mut evaluated = self.clone();

        // Fixed composition: all translations first (equivalent to summing them).
        for track in &self.animation.tracks {
            let AnimationTrack::Translation {
                target,
                interpolation,
                keyframes,
            } = track
            else {
                continue;
            };
            let offset = sample_translation(keyframes, *interpolation, time_seconds);
            apply_translation(&mut evaluated, target, offset);
        }

        // Rotations retain scene declaration order.
        for track in &self.animation.tracks {
            let AnimationTrack::Rotation {
                target,
                axis,
                pivot,
                interpolation,
                keyframes,
            } = track
            else {
                continue;
            };
            let degrees = sample_rotation(keyframes, *interpolation, time_seconds);
            apply_rotation(&mut evaluated, target, axis.normalize(), *pivot, degrees);
        }

        Ok(evaluated)
    }
}

/// Evaluates one group's animation as an absolute renderer-neutral rigid pose.
///
/// The authored origin is translated by the sum of all matching translation
/// tracks, then rotated around each matching world-space pivot in declaration
/// order. This is the same composition used by [`Scene::evaluate_at`].
pub fn evaluate_group_rigid_transform(
    scene: &Scene,
    group: &str,
    authored_origin: Vec3,
    time_seconds: f32,
) -> Result<RigidTransform> {
    GroupRigidTransformEvaluator::new(scene, group, authored_origin)?.evaluate(time_seconds)
}

impl GroupRigidTransformEvaluator {
    /// Captures the validated tracks affecting one existing scene group.
    pub fn new(scene: &Scene, group: &str, authored_origin: Vec3) -> Result<Self> {
        if group.is_empty() {
            bail!("animation target group name must not be empty");
        }
        if !authored_origin.is_finite() {
            bail!("authored rigid-transform origin must be finite");
        }
        validate_animation(scene)?;
        let group_exists = scene
            .spheres
            .iter()
            .any(|sphere| sphere.group.as_deref() == Some(group))
            || scene
                .triangles
                .iter()
                .any(|triangle| triangle.group.as_deref() == Some(group));
        if !group_exists {
            bail!("animation references unknown group '{group}'");
        }
        let tracks = scene
            .animation
            .tracks
            .iter()
            .filter(
                |track| matches!(track.target(), AnimationTarget::Group { name } if name == group),
            )
            .cloned()
            .collect();
        Ok(Self {
            authored_origin,
            tracks,
        })
    }

    /// Samples the captured tracks at one nonnegative scene-local time.
    pub fn evaluate(&self, time_seconds: f32) -> Result<RigidTransform> {
        if !time_seconds.is_finite() || time_seconds < 0.0 {
            bail!("animation time must be finite and non-negative");
        }

        let mut translation = self.authored_origin;
        let mut rotation = Quat::IDENTITY;
        for track in &self.tracks {
            let AnimationTrack::Translation {
                target: AnimationTarget::Group { .. },
                interpolation,
                keyframes,
            } = track
            else {
                continue;
            };
            translation += sample_translation(keyframes, *interpolation, time_seconds);
        }
        for track in &self.tracks {
            let AnimationTrack::Rotation {
                target: AnimationTarget::Group { .. },
                axis,
                pivot,
                interpolation,
                keyframes,
            } = track
            else {
                continue;
            };
            let sampled = Quat::from_axis_angle(
                axis.normalize(),
                sample_rotation(keyframes, *interpolation, time_seconds).to_radians(),
            );
            translation = *pivot + sampled * (translation - *pivot);
            rotation = sampled * rotation;
        }
        Ok(RigidTransform {
            translation,
            rotation,
        })
    }
}

/// Checks track targets, time ordering, finite values, and attribute alignment.
pub(crate) fn validate_animation(scene: &Scene) -> Result<()> {
    if scene.triangle_attributes.len() != scene.triangles.len() {
        bail!("triangle attribute count must match triangle count");
    }
    let groups: HashSet<&str> = scene
        .spheres
        .iter()
        .filter_map(|object| object.group.as_deref())
        .chain(
            scene
                .triangles
                .iter()
                .filter_map(|object| object.group.as_deref()),
        )
        .collect();
    let physics_enabled = scene.physics.is_some_and(|physics| physics.enabled);
    let mut fixed_groups = HashSet::new();
    let mut kinematic_groups = HashMap::<&str, Vec<&ObjectBinding>>::new();
    if physics_enabled {
        for body in &scene.rigid_bodies {
            let Some(group) = body.group.as_deref() else {
                continue;
            };
            match body.body {
                RigidBodyKind::Static | RigidBodyKind::Dynamic => {
                    fixed_groups.insert(group);
                }
                RigidBodyKind::Kinematic => {
                    kinematic_groups
                        .entry(group)
                        .or_default()
                        .push(&body.binding);
                }
            }
        }
    }

    if physics_enabled {
        for (group, bindings) in &kinematic_groups {
            if bindings.len() != 1 {
                bail!(
                    "kinematic animation group '{group}' must identify exactly one physics object"
                );
            }
            validate_exclusive_kinematic_group(scene, group, bindings[0])?;
            if !scene.animation.tracks.iter().any(
                |track| matches!(track.target(), AnimationTarget::Group { name } if name == group),
            ) {
                bail!("kinematic physics group '{group}' requires at least one animation track");
            }
        }
    }

    for track in &scene.animation.tracks {
        if let AnimationTarget::Group { name } = track.target() {
            if name.is_empty() {
                bail!("animation target group name must not be empty");
            }
            if !groups.contains(name.as_str()) {
                bail!("animation references unknown group '{name}'");
            }
            if fixed_groups.contains(name.as_str()) {
                bail!(
                    "animation group '{name}' is controlled by an enabled physics body (static or dynamic)"
                );
            }
        }

        match track {
            AnimationTrack::Translation { keyframes, .. } => {
                validate_times(keyframes.iter().map(|key| key.time))?;
                if keyframes.iter().any(|key| !key.value.is_finite()) {
                    bail!("translation keyframe values must be finite");
                }
            }
            AnimationTrack::Rotation {
                axis,
                pivot,
                keyframes,
                ..
            } => {
                if !axis.is_finite() || axis.length_squared() <= f32::EPSILON {
                    bail!("rotation axis must be finite and non-zero");
                }
                if !pivot.is_finite() {
                    bail!("rotation pivot must be finite");
                }
                validate_times(keyframes.iter().map(|key| key.time))?;
                if keyframes.iter().any(|key| !key.degrees.is_finite()) {
                    bail!("rotation keyframe degrees must be finite");
                }
            }
        }
    }
    Ok(())
}

/// Ensures a kinematic group refers only to the primitives owned by its binding.
fn validate_exclusive_kinematic_group(
    scene: &Scene,
    group: &str,
    binding: &ObjectBinding,
) -> Result<()> {
    match *binding {
        ObjectBinding::Sphere { index } => {
            let sphere_matches =
                scene
                    .spheres
                    .iter()
                    .enumerate()
                    .filter_map(|(candidate, sphere)| {
                        (sphere.group.as_deref() == Some(group)).then_some(candidate)
                    });
            if sphere_matches.ne([index])
                || scene
                    .triangles
                    .iter()
                    .any(|triangle| triangle.group.as_deref() == Some(group))
            {
                bail!("kinematic animation group '{group}' must be exclusive to one source object");
            }
        }
        ObjectBinding::Triangles { start, count, .. } => {
            let end = start
                .checked_add(count)
                .ok_or_else(|| anyhow::anyhow!("kinematic triangle binding overflows"))?;
            if scene
                .spheres
                .iter()
                .any(|sphere| sphere.group.as_deref() == Some(group))
                || scene.triangles.iter().enumerate().any(|(index, triangle)| {
                    triangle.group.as_deref() == Some(group) && !(start..end).contains(&index)
                })
                || scene.triangles.get(start..end).is_none_or(|triangles| {
                    triangles
                        .iter()
                        .any(|triangle| triangle.group.as_deref() != Some(group))
                })
            {
                bail!("kinematic animation group '{group}' must be exclusive to one source object");
            }
        }
    }
    Ok(())
}

/// Ensures a keyframe time sequence is nonempty, finite, nonnegative, and strict.
fn validate_times(times: impl Iterator<Item = f32>) -> Result<()> {
    let mut previous = None;
    let mut count = 0;
    for time in times {
        if !time.is_finite() || time < 0.0 {
            bail!("keyframe times must be finite and non-negative");
        }
        if previous.is_some_and(|previous| time <= previous) {
            bail!("keyframe times must be strictly increasing");
        }
        previous = Some(time);
        count += 1;
    }
    if count == 0 {
        bail!("animation tracks require at least one keyframe");
    }
    Ok(())
}

/// Samples a translation track, clamping outside its keyframe range.
fn sample_translation(
    keys: &[TranslationKeyframe],
    interpolation: Interpolation,
    time: f32,
) -> Vec3 {
    sample_segment(keys, time, |key| key.time)
        .map(|(left, right, amount)| match interpolation {
            Interpolation::Linear => left.value.lerp(right.value, amount),
            Interpolation::Step => left.value,
        })
        .unwrap_or_else(|| endpoint_translation(keys, time))
}

/// Samples a rotation track, clamping outside its keyframe range.
fn sample_rotation(keys: &[RotationKeyframe], interpolation: Interpolation, time: f32) -> f32 {
    sample_segment(keys, time, |key| key.time)
        .map(|(left, right, amount)| match interpolation {
            Interpolation::Linear => left.degrees + (right.degrees - left.degrees) * amount,
            Interpolation::Step => left.degrees,
        })
        .unwrap_or_else(|| endpoint_rotation(keys, time))
}

/// Locates the adjacent keyframes containing `time` and returns their blend amount.
fn sample_segment<T>(keys: &[T], time: f32, key_time: impl Fn(&T) -> f32) -> Option<(&T, &T, f32)> {
    for pair in keys.windows(2) {
        let start = key_time(&pair[0]);
        let end = key_time(&pair[1]);
        if time >= start && time < end {
            return Some((&pair[0], &pair[1], (time - start) / (end - start)));
        }
    }
    None
}

/// Returns the first or last translation when time lies outside the key range.
///
/// The caller guarantees that `keys` is nonempty through animation validation.
fn endpoint_translation(keys: &[TranslationKeyframe], time: f32) -> Vec3 {
    if time < keys[0].time {
        keys[0].value
    } else {
        keys[keys.len() - 1].value
    }
}

/// Returns the first or last rotation when time lies outside the key range.
///
/// The caller guarantees that `keys` is nonempty through animation validation.
fn endpoint_rotation(keys: &[RotationKeyframe], time: f32) -> f32 {
    if time < keys[0].time {
        keys[0].degrees
    } else {
        keys[keys.len() - 1].degrees
    }
}

/// Applies a sampled offset to the camera or every primitive in a group.
fn apply_translation(scene: &mut Scene, target: &AnimationTarget, offset: Vec3) {
    match target {
        AnimationTarget::Camera => {
            scene.camera.position += offset;
            scene.camera.look_at += offset;
        }
        AnimationTarget::Group { name } => {
            for sphere in &mut scene.spheres {
                if sphere.group.as_deref() == Some(name) {
                    sphere.center += offset;
                }
            }
            for triangle in &mut scene.triangles {
                if triangle.group.as_deref() == Some(name) {
                    for vertex in &mut triangle.vertices {
                        *vertex += offset;
                    }
                }
            }
        }
    }
}

/// Applies an axis/pivot rotation to camera vectors or grouped geometry.
///
/// Smooth triangle normals rotate with their vertices; texture coordinates are
/// invariant under geometric transforms.
fn apply_rotation(
    scene: &mut Scene,
    target: &AnimationTarget,
    axis: Vec3,
    pivot: Vec3,
    degrees: f32,
) {
    let rotation = Quat::from_axis_angle(axis, degrees.to_radians());
    let rotate_point = |point: Vec3| pivot + rotation * (point - pivot);

    match target {
        AnimationTarget::Camera => {
            scene.camera.position = rotate_point(scene.camera.position);
            scene.camera.look_at = rotate_point(scene.camera.look_at);
            scene.camera.up = rotation * scene.camera.up;
        }
        AnimationTarget::Group { name } => {
            for sphere in &mut scene.spheres {
                if sphere.group.as_deref() == Some(name) {
                    sphere.center = rotate_point(sphere.center);
                }
            }
            for (triangle, attributes) in scene
                .triangles
                .iter_mut()
                .zip(&mut scene.triangle_attributes)
            {
                if triangle.group.as_deref() == Some(name) {
                    for vertex in &mut triangle.vertices {
                        *vertex = rotate_point(*vertex);
                    }
                    if let Some(normals) = &mut attributes.normals {
                        for normal in normals {
                            *normal = rotation * *normal;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Background, CameraSettings, Material, RenderSettings, Sphere, Triangle};

    fn base_scene(tracks: Vec<AnimationTrack>) -> Scene {
        Scene {
            camera: CameraSettings {
                position: Vec3::new(0.0, 0.0, 2.0),
                look_at: Vec3::ZERO,
                up: Vec3::Y,
                fov_degrees: 45.0,
            },
            render: RenderSettings {
                width: 1,
                height: 1,
                samples: 1,
                max_bounces: 1,
                background: Background::Sky,
            },
            display: Default::default(),
            materials: vec![Material::Diffuse { albedo: Vec3::ONE }],
            spheres: vec![
                Sphere {
                    center: Vec3::X,
                    radius: 0.25,
                    material_index: 0,
                    group: Some("moving".into()),
                },
                Sphere {
                    center: -Vec3::X,
                    radius: 0.25,
                    material_index: 0,
                    group: None,
                },
            ],
            triangles: vec![Triangle {
                vertices: [Vec3::X, Vec3::X + Vec3::Y, Vec3::X + Vec3::Z],
                material_index: 0,
                group: Some("moving".into()),
            }],
            triangle_attributes: vec![crate::TriangleAttributes::default()],
            textures: Vec::new(),
            environment: None,
            animation: Animation { tracks },
            physics: None,
            rigid_bodies: Vec::new(),
            triggers: Vec::new(),
            spawn_points: Vec::new(),
            event_reactions: Vec::new(),
        }
    }

    fn translation(
        interpolation: Interpolation,
        keyframes: Vec<TranslationKeyframe>,
    ) -> AnimationTrack {
        AnimationTrack::Translation {
            target: AnimationTarget::Group {
                name: "moving".into(),
            },
            interpolation,
            keyframes,
        }
    }

    fn rotation(target: AnimationTarget, degrees: f32) -> AnimationTrack {
        AnimationTrack::Rotation {
            target,
            axis: Vec3::Y,
            pivot: Vec3::ZERO,
            interpolation: Interpolation::Linear,
            keyframes: vec![RotationKeyframe { time: 0.0, degrees }],
        }
    }

    #[test]
    fn interpolates_linearly_and_holds_endpoints() {
        let scene = base_scene(vec![translation(
            Interpolation::Linear,
            vec![
                TranslationKeyframe {
                    time: 1.0,
                    value: Vec3::ZERO,
                },
                TranslationKeyframe {
                    time: 3.0,
                    value: Vec3::new(4.0, 0.0, 0.0),
                },
            ],
        )]);

        assert_eq!(scene.evaluate_at(0.0).unwrap().spheres[0].center, Vec3::X);
        assert_eq!(
            scene.evaluate_at(2.0).unwrap().spheres[0].center,
            Vec3::new(3.0, 0.0, 0.0)
        );
        assert_eq!(
            scene.evaluate_at(5.0).unwrap().spheres[0].center,
            Vec3::new(5.0, 0.0, 0.0)
        );
    }

    #[test]
    fn step_interpolation_switches_at_keyframe() {
        let scene = base_scene(vec![translation(
            Interpolation::Step,
            vec![
                TranslationKeyframe {
                    time: 0.0,
                    value: Vec3::ZERO,
                },
                TranslationKeyframe {
                    time: 1.0,
                    value: Vec3::Y,
                },
            ],
        )]);

        assert_eq!(scene.evaluate_at(0.99).unwrap().spheres[0].center, Vec3::X);
        assert_eq!(
            scene.evaluate_at(1.0).unwrap().spheres[0].center,
            Vec3::X + Vec3::Y
        );
    }

    #[test]
    fn sums_translations_then_applies_rotations() {
        let offset = TranslationKeyframe {
            time: 0.0,
            value: Vec3::new(0.5, 0.0, 0.0),
        };
        let scene = base_scene(vec![
            rotation(
                AnimationTarget::Group {
                    name: "moving".into(),
                },
                90.0,
            ),
            translation(Interpolation::Linear, vec![offset]),
            translation(Interpolation::Step, vec![offset]),
        ]);
        let evaluated = scene.evaluate_at(0.0).unwrap();

        assert!(evaluated.spheres[0]
            .center
            .abs_diff_eq(Vec3::new(0.0, 0.0, -2.0), 1e-5));
        assert_eq!(evaluated.spheres[1].center, -Vec3::X);
        assert_eq!(scene.spheres[0].center, Vec3::X);
    }

    #[test]
    fn neutral_group_transform_matches_geometry_animation() {
        let offset = TranslationKeyframe {
            time: 0.0,
            value: Vec3::new(0.5, 0.0, 0.0),
        };
        let scene = base_scene(vec![
            translation(Interpolation::Linear, vec![offset]),
            rotation(
                AnimationTarget::Group {
                    name: "moving".into(),
                },
                90.0,
            ),
        ]);
        let pose = evaluate_group_rigid_transform(&scene, "moving", Vec3::X, 0.0).unwrap();
        let evaluated = scene.evaluate_at(0.0).unwrap();

        assert!(pose
            .translation
            .abs_diff_eq(evaluated.spheres[0].center, 1.0e-5));
        assert!((pose.rotation * Vec3::Y).abs_diff_eq(Vec3::Y, 1.0e-5));
        assert_eq!(scene.spheres[0].center, Vec3::X);
    }

    #[test]
    fn transforms_grouped_triangles_and_spheres_only() {
        let scene = base_scene(vec![translation(
            Interpolation::Linear,
            vec![TranslationKeyframe {
                time: 0.0,
                value: Vec3::Y,
            }],
        )]);
        let evaluated = scene.evaluate_at(0.0).unwrap();

        assert_eq!(evaluated.spheres[0].center, Vec3::X + Vec3::Y);
        assert_eq!(evaluated.spheres[1].center, -Vec3::X);
        assert_eq!(evaluated.triangles[0].vertices[0], Vec3::X + Vec3::Y);
    }

    #[test]
    fn camera_rotation_about_look_at_reproduces_orbit() {
        let scene = base_scene(vec![rotation(AnimationTarget::Camera, 90.0)]);
        let camera = scene.evaluate_at(0.0).unwrap().camera;

        assert!(camera.position.abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-5));
        assert!(camera.look_at.abs_diff_eq(Vec3::ZERO, 1e-5));
        assert!(camera.up.abs_diff_eq(Vec3::Y, 1e-5));
    }

    #[test]
    fn rejects_invalid_tracks_and_time() {
        let mut scene = base_scene(vec![rotation(
            AnimationTarget::Group {
                name: "missing".into(),
            },
            0.0,
        )]);
        assert!(scene.evaluate_at(0.0).is_err());

        scene.animation.tracks = vec![AnimationTrack::Rotation {
            target: AnimationTarget::Camera,
            axis: Vec3::ZERO,
            pivot: Vec3::ZERO,
            interpolation: Interpolation::Linear,
            keyframes: vec![RotationKeyframe {
                time: 0.0,
                degrees: 0.0,
            }],
        }];
        assert!(scene.evaluate_at(0.0).is_err());
        assert!(base_scene(Vec::new()).evaluate_at(f32::NAN).is_err());
    }
}
