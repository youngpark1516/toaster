//! Renderer-neutral equirectangular environment-map storage and sampling.

use crate::texture::srgb_to_linear;
use anyhow::{ensure, Context, Result};
use glam::Vec3;
use image::{ImageFormat, ImageReader};
use std::{f32::consts::PI, path::Path, sync::Arc};

#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentMap {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[Vec3]>,
    pub intensity: f32,
    pub rotation_degrees: f32,
}

impl EnvironmentMap {
    pub fn new(
        width: u32,
        height: u32,
        pixels: impl Into<Arc<[Vec3]>>,
        intensity: f32,
        rotation_degrees: f32,
    ) -> Result<Self> {
        let pixels = pixels.into();
        ensure!(
            width > 0 && height > 0,
            "environment dimensions must be nonzero"
        );
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .context("environment dimensions are too large")?;
        ensure!(
            pixels.len() == expected_len,
            "environment contains {} pixels but {} were expected",
            pixels.len(),
            expected_len
        );
        ensure!(
            pixels
                .iter()
                .all(|pixel| pixel.is_finite() && pixel.cmpge(Vec3::ZERO).all()),
            "environment pixels must be finite and non-negative"
        );
        ensure!(
            intensity.is_finite() && intensity >= 0.0,
            "environment intensity must be finite and non-negative"
        );
        ensure!(
            rotation_degrees.is_finite(),
            "environment rotation_degrees must be finite"
        );

        Ok(Self {
            width,
            height,
            pixels,
            intensity,
            rotation_degrees: rotation_degrees.rem_euclid(360.0),
        })
    }

    pub fn load(path: impl AsRef<Path>, intensity: f32, rotation_degrees: f32) -> Result<Self> {
        let path = path.as_ref();
        let reader = ImageReader::open(path)
            .with_context(|| format!("failed to open environment map {}", path.display()))?
            .with_guessed_format()
            .with_context(|| {
                format!("failed to detect environment map format {}", path.display())
            })?;
        let is_hdr = reader.format() == Some(ImageFormat::Hdr);
        let image = reader
            .decode()
            .with_context(|| format!("failed to decode environment map {}", path.display()))?;
        let width = image.width();
        let height = image.height();
        let pixels: Vec<Vec3> = if is_hdr {
            image
                .to_rgb32f()
                .pixels()
                .map(|pixel| Vec3::from_array(pixel.0))
                .collect()
        } else {
            image
                .to_rgb8()
                .pixels()
                .map(|pixel| {
                    Vec3::new(
                        srgb_to_linear(pixel[0]),
                        srgb_to_linear(pixel[1]),
                        srgb_to_linear(pixel[2]),
                    )
                })
                .collect()
        };
        Self::new(width, height, pixels, intensity, rotation_degrees)
    }

    /// Samples a latitude-longitude map with horizontal wrapping, vertical
    /// clamping, and bilinear filtering. Rotation is a yaw in degrees.
    pub fn sample(&self, direction: Vec3) -> Vec3 {
        let direction = direction.normalize_or_zero();
        if direction == Vec3::ZERO {
            return Vec3::ZERO;
        }
        let u = (direction.z.atan2(direction.x) / (2.0 * PI) + 0.5 + self.rotation_degrees / 360.0)
            .rem_euclid(1.0);
        let v = direction.y.clamp(-1.0, 1.0).acos() / PI;
        let x = u * self.width as f32 - 0.5;
        let y = v * self.height as f32 - 0.5;
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let amount_x = x - x.floor();
        let amount_y = y - y.floor();

        let top = self.texel(x0, y0).lerp(self.texel(x0 + 1, y0), amount_x);
        let bottom = self
            .texel(x0, y0 + 1)
            .lerp(self.texel(x0 + 1, y0 + 1), amount_x);
        top.lerp(bottom, amount_y) * self.intensity
    }

    fn texel(&self, x: i32, y: i32) -> Vec3 {
        let x = x.rem_euclid(self.width as i32) as usize;
        let y = y.clamp(0, self.height as i32 - 1) as usize;
        self.pixels[y * self.width as usize + x]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::hdr::HdrEncoder, Rgb};
    use std::{fs::File, io::BufWriter};

    #[test]
    fn samples_directions_rotation_and_intensity() {
        let environment =
            EnvironmentMap::new(4, 1, vec![Vec3::X, Vec3::Y, Vec3::Z, Vec3::ONE], 2.0, 0.0)
                .unwrap();
        let direction_at_u = |u: f32| {
            let angle = (u - 0.5) * 2.0 * PI;
            Vec3::new(angle.cos(), 0.0, angle.sin())
        };

        assert!(environment
            .sample(direction_at_u(0.625))
            .abs_diff_eq(Vec3::Z * 2.0, 1e-6));
        assert!(environment
            .sample(direction_at_u(0.375))
            .abs_diff_eq(Vec3::Y * 2.0, 1e-6));

        let rotated = EnvironmentMap::new(4, 1, environment.pixels, 1.0, 90.0).unwrap();
        assert!(rotated
            .sample(direction_at_u(0.375))
            .abs_diff_eq(Vec3::Z, 1e-6));
    }

    #[test]
    fn loads_hdr_without_clamping_high_dynamic_range() {
        let path = std::env::temp_dir().join(format!(
            "toaster-environment-{}-{}.hdr",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let file = File::create(&path).unwrap();
        HdrEncoder::new(BufWriter::new(file))
            .encode(&[Rgb([4.0, 2.0, 1.0])], 1, 1)
            .unwrap();

        let environment = EnvironmentMap::load(&path, 0.5, -90.0).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!((environment.width, environment.height), (1, 1));
        assert!((environment.pixels[0] - Vec3::new(4.0, 2.0, 1.0)).length() < 0.05);
        assert_eq!(environment.rotation_degrees, 270.0);
        assert!((environment.sample(Vec3::X) - Vec3::new(2.0, 1.0, 0.5)).length() < 0.05);
    }

    #[test]
    fn loads_ldr_environment_as_linear_srgb() {
        let path = std::env::temp_dir().join(format!(
            "toaster-environment-{}-ldr.png",
            std::process::id()
        ));
        image::RgbImage::from_raw(1, 1, vec![128, 0, 255])
            .unwrap()
            .save(&path)
            .unwrap();

        let environment = EnvironmentMap::load(&path, 1.0, 0.0).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(environment.pixels[0].abs_diff_eq(Vec3::new(0.21586, 0.0, 1.0), 1e-4));
    }

    #[test]
    fn rejects_invalid_dimensions_pixels_and_controls() {
        assert!(EnvironmentMap::new(0, 1, Vec::new(), 1.0, 0.0).is_err());
        assert!(EnvironmentMap::new(1, 1, vec![Vec3::ONE; 2], 1.0, 0.0).is_err());
        assert!(EnvironmentMap::new(1, 1, vec![-Vec3::ONE], 1.0, 0.0).is_err());
        assert!(EnvironmentMap::new(1, 1, vec![Vec3::ONE], -1.0, 0.0).is_err());
        assert!(EnvironmentMap::new(1, 1, vec![Vec3::ONE], 1.0, f32::NAN).is_err());
    }
}
