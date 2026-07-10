use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct AnimationConfig {
    pub fps: u32,
    pub duration_seconds: f32,
    pub orbit_degrees: Option<f32>,
}

impl AnimationConfig {
    pub fn single_frame() -> Self {
        Self {
            fps: 1,
            duration_seconds: 1.0,
            orbit_degrees: None,
        }
    }

    pub fn frame_count(&self) -> u32 {
        let frames = (self.fps as f32 * self.duration_seconds).ceil() as u32;
        frames.max(1)
    }

    pub fn time_for_frame(&self, frame: u32) -> f32 {
        frame as f32 / self.fps as f32
    }
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
