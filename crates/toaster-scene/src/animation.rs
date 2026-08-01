//! Renderer-independent scene animation tracks and evaluation.

use crate::Scene;
use anyhow::{bail, Result};
use glam::{Quat, Vec3};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Animation {
    #[serde(default)]
    pub tracks: Vec<AnimationTrack>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnimationTarget {
    Group { name: String },
    Camera,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Linear,
    Step,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct TranslationKeyframe {
    pub time: f32,
    pub value: Vec3,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct RotationKeyframe {
    pub time: f32,
    pub degrees: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnimationTrack {
    Translation {
        target: AnimationTarget,
        interpolation: Interpolation,
        keyframes: Vec<TranslationKeyframe>,
    },
    Rotation {
        target: AnimationTarget,
        axis: Vec3,
        pivot: Vec3,
        interpolation: Interpolation,
        keyframes: Vec<RotationKeyframe>,
    },
}

impl AnimationTrack {
    pub fn target(&self) -> &AnimationTarget {
        match self {
            Self::Translation { target, .. } | Self::Rotation { target, .. } => target,
        }
    }
}

impl Animation {
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

    for track in &scene.animation.tracks {
        if let AnimationTarget::Group { name } = track.target() {
            if name.is_empty() {
                bail!("animation target group name must not be empty");
            }
            if !groups.contains(name.as_str()) {
                bail!("animation references unknown group '{name}'");
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

fn sample_rotation(keys: &[RotationKeyframe], interpolation: Interpolation, time: f32) -> f32 {
    sample_segment(keys, time, |key| key.time)
        .map(|(left, right, amount)| match interpolation {
            Interpolation::Linear => left.degrees + (right.degrees - left.degrees) * amount,
            Interpolation::Step => left.degrees,
        })
        .unwrap_or_else(|| endpoint_rotation(keys, time))
}

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

fn endpoint_translation(keys: &[TranslationKeyframe], time: f32) -> Vec3 {
    if time < keys[0].time {
        keys[0].value
    } else {
        keys[keys.len() - 1].value
    }
}

fn endpoint_rotation(keys: &[RotationKeyframe], time: f32) -> f32 {
    if time < keys[0].time {
        keys[0].degrees
    } else {
        keys[keys.len() - 1].degrees
    }
}

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
            animation: Animation { tracks },
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
