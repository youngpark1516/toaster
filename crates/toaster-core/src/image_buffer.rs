//! Dense linear-RGB image storage used by the CPU reference renderer.

use crate::color::{linear_hdr_to_rgb8, DisplaySettings};
use anyhow::{Context, Result};
use glam::Vec3;
use image::{Rgb, RgbImage};
use std::path::Path;

#[derive(Clone, Debug)]
/// A row-major image whose pixels remain in linear floating-point RGB.
pub struct ImageBuffer {
    width: u32,
    height: u32,
    pixels: Vec<Vec3>,
}

impl ImageBuffer {
    /// Allocates a black image with the requested dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![Vec3::ZERO; (width * height) as usize],
        }
    }

    /// Returns the image width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the image height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Replaces the pixel at `(x, y)` with a linear RGB value.
    ///
    /// # Panics
    ///
    /// Panics when either coordinate is outside the image.
    pub fn set_pixel(&mut self, x: u32, y: u32, color: Vec3) {
        self.pixels[(y * self.width + x) as usize] = color;
    }

    /// Returns the linear RGB value at `(x, y)`.
    ///
    /// # Panics
    ///
    /// Panics when either coordinate is outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Vec3 {
        self.pixels[(y * self.width + x) as usize]
    }

    /// Converts the image to display RGB and saves it as a PNG-compatible image.
    ///
    /// Missing parent directories are created. Errors include directory creation
    /// and image-encoding failures.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<()> {
        self.save_png_with_display(path, DisplaySettings::default())
    }

    /// Converts the image with `display` and saves it as a PNG-compatible image.
    pub fn save_png_with_display(
        &self,
        path: impl AsRef<Path>,
        display: DisplaySettings,
    ) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create output directory {}", parent.display())
            })?;
        }

        let mut image = RgbImage::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                image.put_pixel(x, y, Rgb(linear_hdr_to_rgb8(self.pixel(x, y), display)));
            }
        }
        image
            .save(path)
            .with_context(|| format!("failed to save PNG {}", path.display()))
    }
}
