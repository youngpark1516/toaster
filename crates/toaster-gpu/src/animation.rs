use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct AnimationConfig {
    fps: Option<u32>,
    frame_limit: Option<u32>,
    loop_duration_seconds: Option<f32>,
}

impl AnimationConfig {
    pub fn single_frame() -> Self {
        Self {
            fps: None,
            frame_limit: Some(1),
            loop_duration_seconds: None,
        }
    }

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

    pub fn indefinite(fps: u32) -> Result<Self> {
        validate_fps(fps)?;
        Ok(Self {
            fps: Some(fps),
            frame_limit: None,
            loop_duration_seconds: None,
        })
    }

    pub fn with_loop_duration(mut self, duration_seconds: f32) -> Result<Self> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            bail!("loop duration must be finite and greater than zero");
        }
        self.loop_duration_seconds = Some(duration_seconds);
        Ok(self)
    }

    pub fn frame_limit(&self) -> Option<u32> {
        self.frame_limit
    }

    pub fn time_for_frame(&self, frame: u32) -> f32 {
        let time = self.fps.map_or(0.0, |fps| frame as f32 / fps as f32);
        self.loop_duration_seconds
            .map_or(time, |duration| time % duration)
    }

    pub fn fps(&self) -> Option<u32> {
        self.fps
    }

    pub fn loop_duration(&self) -> Option<f32> {
        self.loop_duration_seconds
    }
}

fn validate_fps(fps: u32) -> Result<()> {
    if fps == 0 {
        bail!("fps must be greater than zero");
    }
    Ok(())
}

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
