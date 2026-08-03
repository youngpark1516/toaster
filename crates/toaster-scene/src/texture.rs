//! Renderer-neutral base-color texture storage and sampling.

use anyhow::{ensure, Result};
use glam::{Vec2, Vec3};

#[derive(Clone, Debug, PartialEq, Eq)]
/// A tightly packed RGBA8 base-color texture.
pub struct Texture {
    /// Texture width in texels.
    pub width: u32,
    /// Texture height in texels.
    pub height: u32,
    /// Row-major RGBA8 texels; alpha is retained but currently ignored by shading.
    pub rgba8: Vec<u8>,
}

impl Texture {
    /// Validates dimensions and byte length before constructing a texture.
    ///
    /// ```
    /// use glam::Vec2;
    /// use toaster_scene::Texture;
    ///
    /// let texture = Texture::new(1, 1, vec![255, 0, 0, 255])?;
    /// let red = texture.sample_linear(Vec2::ZERO);
    /// assert_eq!(red.to_array(), [1.0, 0.0, 0.0]);
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn new(width: u32, height: u32, rgba8: Vec<u8>) -> Result<Self> {
        ensure!(
            width > 0 && height > 0,
            "texture dimensions must be nonzero"
        );
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| anyhow::anyhow!("texture dimensions are too large"))?;
        ensure!(
            rgba8.len() == expected_len,
            "texture contains {} bytes but {} were expected",
            rgba8.len(),
            expected_len
        );
        Ok(Self {
            width,
            height,
            rgba8,
        })
    }

    /// Samples repeating normalized UVs with bilinear filtering and converts
    /// the stored sRGB base color into linear RGB.
    pub fn sample_linear(&self, uv: Vec2) -> Vec3 {
        let x = uv.x.rem_euclid(1.0) * self.width as f32 - 0.5;
        let y = uv.y.rem_euclid(1.0) * self.height as f32 - 0.5;
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let amount_x = x - x.floor();
        let amount_y = y - y.floor();

        let top = self
            .texel_linear(x0, y0)
            .lerp(self.texel_linear(x0 + 1, y0), amount_x);
        let bottom = self
            .texel_linear(x0, y0 + 1)
            .lerp(self.texel_linear(x0 + 1, y0 + 1), amount_x);
        top.lerp(bottom, amount_y)
    }

    /// Fetches one wrapped texel and converts its RGB channels from sRGB.
    fn texel_linear(&self, x: i32, y: i32) -> Vec3 {
        let wrapped_x = x.rem_euclid(self.width as i32) as usize;
        let wrapped_y = y.rem_euclid(self.height as i32) as usize;
        let offset = (wrapped_y * self.width as usize + wrapped_x) * 4;
        Vec3::new(
            srgb_to_linear(self.rgba8[offset]),
            srgb_to_linear(self.rgba8[offset + 1]),
            srgb_to_linear(self.rgba8[offset + 2]),
        )
    }
}

/// Converts one 8-bit sRGB channel to linear intensity.
pub(crate) fn srgb_to_linear(value: u8) -> f32 {
    let value = value as f32 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_size_and_samples_srgb_texel_centers() {
        assert!(Texture::new(1, 1, vec![0; 3]).is_err());
        let texture = Texture::new(2, 1, vec![255, 0, 0, 255, 0, 128, 0, 255]).unwrap();

        assert_eq!(texture.sample_linear(Vec2::new(0.25, 0.5)), Vec3::X);
        let green = texture.sample_linear(Vec2::new(0.75, 0.5));
        assert_eq!(green.x, 0.0);
        assert!((green.y - 0.21586).abs() < 1e-4);
        assert_eq!(green.z, 0.0);
    }
}
