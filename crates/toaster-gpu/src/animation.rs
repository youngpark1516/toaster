//! Render frame schedules and numbered output paths.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use toaster_scene::EvaluationRequest;

#[derive(Debug, Clone, Copy)]
/// A validated finite or indefinite render schedule.
pub struct AnimationConfig {
    /// Frames per second; absent only for the single frame at time zero.
    fps: Option<u32>,
    /// Exact frame limit; absent for indefinite preview.
    frame_limit: Option<u32>,
    /// Optional animation-time wrapping interval.
    loop_duration_seconds: Option<f32>,
}

impl AnimationConfig {
    /// Creates one frame at animation time zero without an FPS.
    pub fn single_frame() -> Self {
        Self {
            fps: None,
            frame_limit: Some(1),
            loop_duration_seconds: None,
        }
    }

    /// Creates `ceil(fps * duration_seconds)` frames.
    ///
    /// ```
    /// use toaster_gpu::AnimationConfig;
    ///
    /// let schedule = AnimationConfig::from_duration(24, 1.1)?;
    /// assert_eq!(schedule.frame_limit(), Some(27));
    /// assert_eq!(schedule.time_for_frame(12), 0.5);
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn from_duration(fps: u32, duration_seconds: f32) -> Result<Self> {
        validate_fps(fps)?;
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            bail!("duration must be finite and greater than zero");
        }
        let frame_count = (fps as f64 * duration_seconds as f64).ceil();
        if frame_count > u32::MAX as f64 {
            bail!("animation frame count is too large");
        }
        Ok(Self {
            fps: Some(fps),
            frame_limit: Some(frame_count as u32),
            loop_duration_seconds: None,
        })
    }

    /// Creates an exact positive frame count at the selected FPS.
    pub fn from_frame_count(fps: u32, frame_count: u32) -> Result<Self> {
        validate_fps(fps)?;
        if frame_count == 0 {
            bail!("frames must be greater than zero");
        }
        Ok(Self {
            fps: Some(fps),
            frame_limit: Some(frame_count),
            loop_duration_seconds: None,
        })
    }

    /// Creates an unbounded frame schedule at the selected FPS.
    pub fn indefinite(fps: u32) -> Result<Self> {
        validate_fps(fps)?;
        Ok(Self {
            fps: Some(fps),
            frame_limit: None,
            loop_duration_seconds: None,
        })
    }

    /// Adds a positive animation-time wrapping interval without changing indices.
    pub fn with_loop_duration(mut self, duration_seconds: f32) -> Result<Self> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            bail!("loop duration must be finite and greater than zero");
        }
        self.loop_duration_seconds = Some(duration_seconds);
        Ok(self)
    }

    /// Returns the exact frame limit, or `None` for an indefinite schedule.
    pub fn frame_limit(&self) -> Option<u32> {
        self.frame_limit
    }

    /// Maps an index to animation seconds and applies optional loop wrapping.
    pub fn time_for_frame(&self, frame: u32) -> f32 {
        self.evaluation_request(frame).time_seconds
    }

    /// Maps an index to wrapped scene time and an explicit loop cycle.
    pub fn evaluation_request(&self, frame: u32) -> EvaluationRequest {
        let absolute_time = self.fps.map_or(0.0_f64, |fps| frame as f64 / fps as f64);
        match self.loop_duration_seconds {
            Some(duration) => {
                let duration = duration as f64;
                let loop_cycle = (absolute_time / duration).floor();
                EvaluationRequest {
                    time_seconds: (absolute_time - loop_cycle * duration) as f32,
                    loop_cycle: loop_cycle as u64,
                }
            }
            None => EvaluationRequest {
                time_seconds: absolute_time as f32,
                loop_cycle: 0,
            },
        }
    }

    /// Returns the schedule FPS, absent for [`Self::single_frame`].
    pub fn fps(&self) -> Option<u32> {
        self.fps
    }

    /// Returns the optional animation-time wrapping interval.
    pub fn loop_duration(&self) -> Option<f32> {
        self.loop_duration_seconds
    }
}

/// Rejects zero FPS values shared by all animated constructors.
fn validate_fps(fps: u32) -> Result<()> {
    if fps == 0 {
        bail!("fps must be greater than zero");
    }
    Ok(())
}

/// Returns the original output path for one frame or a zero-padded numbered path.
pub fn frame_output_path(out_path: &Path, frame: u32, frame_count: u32) -> PathBuf {
    if frame_count == 1 {
        return out_path.to_path_buf();
    }

    let parent = out_path.parent().unwrap_or_else(|| Path::new(""));

    let stem = out_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("frame");

    let ext = out_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("png");

    parent.join(format!("{}_{:04}.{}", stem, frame, ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_uses_ceiling_and_explicit_fps() {
        let animation = AnimationConfig::from_duration(24, 1.01).unwrap();
        assert_eq!(animation.frame_limit(), Some(25));
        assert_eq!(animation.time_for_frame(12), 0.5);
    }

    #[test]
    fn frame_count_is_exact() {
        let animation = AnimationConfig::from_frame_count(30, 17).unwrap();
        assert_eq!(animation.frame_limit(), Some(17));
        assert_eq!(animation.fps(), Some(30));
    }

    #[test]
    fn single_frame_has_no_inferred_fps() {
        let animation = AnimationConfig::single_frame();
        assert_eq!(animation.fps(), None);
        assert_eq!(animation.frame_limit(), Some(1));
        assert_eq!(animation.time_for_frame(0), 0.0);
    }

    #[test]
    fn indefinite_animation_has_no_frame_limit() {
        let animation = AnimationConfig::indefinite(12).unwrap();
        assert_eq!(animation.fps(), Some(12));
        assert_eq!(animation.frame_limit(), None);
        assert_eq!(animation.time_for_frame(24), 2.0);
    }

    #[test]
    fn loop_duration_wraps_animation_time_without_wrapping_frame_count() {
        let animation = AnimationConfig::indefinite(4)
            .unwrap()
            .with_loop_duration(2.0)
            .unwrap();

        assert_eq!(animation.time_for_frame(7), 1.75);
        assert_eq!(animation.time_for_frame(8), 0.0);
        assert_eq!(animation.evaluation_request(8).loop_cycle, 1);
        assert_eq!(animation.time_for_frame(10), 0.5);
        assert_eq!(animation.frame_limit(), None);
        assert_eq!(animation.loop_duration(), Some(2.0));
    }

    #[test]
    fn rejects_invalid_timing() {
        assert!(AnimationConfig::from_duration(0, 1.0).is_err());
        assert!(AnimationConfig::from_duration(24, f32::NAN).is_err());
        assert!(AnimationConfig::from_duration(24, 0.0).is_err());
        assert!(AnimationConfig::from_frame_count(24, 0).is_err());
        assert!(AnimationConfig::indefinite(0).is_err());
        assert!(AnimationConfig::indefinite(24)
            .unwrap()
            .with_loop_duration(f32::NAN)
            .is_err());
        assert!(AnimationConfig::indefinite(24)
            .unwrap()
            .with_loop_duration(0.0)
            .is_err());
    }
}
