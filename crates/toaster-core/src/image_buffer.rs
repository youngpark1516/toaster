use crate::color::linear_to_rgb8;
use anyhow::{Context, Result};
use glam::Vec3;
use image::{Rgb, RgbImage};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct ImageBuffer {
    width: u32,
    height: u32,
    pixels: Vec<Vec3>,
}

impl ImageBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![Vec3::ZERO; (width * height) as usize],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, color: Vec3) {
        self.pixels[(y * self.width + x) as usize] = color;
    }

    pub fn pixel(&self, x: u32, y: u32) -> Vec3 {
        self.pixels[(y * self.width + x) as usize]
    }

    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<()> {
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
                image.put_pixel(x, y, Rgb(linear_to_rgb8(self.pixel(x, y))));
            }
        }
        image
            .save(path)
            .with_context(|| format!("failed to save PNG {}", path.display()))
    }
}
