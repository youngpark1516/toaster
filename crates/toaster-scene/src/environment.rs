//! Renderer-neutral equirectangular environment-map storage and sampling.

use crate::texture::srgb_to_linear;
use anyhow::{ensure, Context, Result};
use glam::Vec3;
use image::{ImageFormat, ImageReader};
use std::{f32::consts::PI, path::Path, sync::Arc};

#[derive(Clone, Debug, PartialEq)]
/// A validated linear-radiance latitude-longitude environment map.
pub struct EnvironmentMap {
    /// Map width in texels.
    pub width: u32,
    /// Map height in texels.
    pub height: u32,
    /// Row-major linear RGB radiance before the intensity multiplier.
    pub pixels: Arc<[Vec3]>,
    /// Nonnegative radiance multiplier.
    pub intensity: f32,
    /// Normalized yaw rotation in degrees.
    pub rotation_degrees: f32,
    /// Luminance-and-latitude weighted cumulative distribution.
    importance_cdf: Arc<[f32]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// One direction drawn from an environment map's importance distribution.
pub struct EnvironmentSample {
    /// Normalized world-space direction toward the environment.
    pub direction: Vec3,
    /// Bilinearly filtered radiance along `direction`.
    pub radiance: Vec3,
    /// Sampling probability density per steradian.
    pub pdf_solid_angle: f32,
}

impl EnvironmentMap {
    /// Validates map data and builds its importance-sampling distribution.
    ///
    /// Dimensions must be nonzero, pixel count must match, radiance must be
    /// finite and nonnegative, and intensity must be finite and nonnegative.
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

        let importance_cdf = build_importance_distribution(width, height, &pixels, intensity);

        Ok(Self {
            width,
            height,
            pixels,
            intensity,
            rotation_degrees: rotation_degrees.rem_euclid(360.0),
            importance_cdf,
        })
    }

    /// Loads HDR radiance or converts an LDR image from sRGB before validation.
    ///
    /// Errors include file access, format detection, decoding, and invalid map
    /// data reported by [`Self::new`].
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

    /// Draws a direction from the luminance-weighted lat-long distribution.
    ///
    /// `selection`, `jitter_u`, and `jitter_v` are expected to be uniform random
    /// values in `[0, 1)`. Keeping randomness outside this renderer-neutral type
    /// lets the CPU and GPU use exactly the same distribution.
    pub fn sample_importance(
        &self,
        selection: f32,
        jitter_u: f32,
        jitter_v: f32,
    ) -> Option<EnvironmentSample> {
        if self.importance_cdf.last().copied().unwrap_or(0.0) <= 0.0 {
            return None;
        }

        let selection = unit_interval(selection);
        let index = self
            .importance_cdf
            .partition_point(|&cumulative| cumulative <= selection)
            .min(self.importance_cdf.len() - 1);
        let x = index % self.width as usize;
        let y = index / self.width as usize;
        let u = (x as f32 + unit_interval(jitter_u)) / self.width as f32;
        let v = (y as f32 + unit_interval(jitter_v)) / self.height as f32;
        let theta = PI * v;
        let phi = 2.0 * PI * (u - 0.5 - self.rotation_degrees / 360.0);
        let sin_theta = theta.sin();
        let direction = Vec3::new(sin_theta * phi.cos(), theta.cos(), sin_theta * phi.sin());
        let pdf_solid_angle = self.texel_pdf_solid_angle(index, sin_theta);

        Some(EnvironmentSample {
            direction,
            radiance: self.sample(direction),
            pdf_solid_angle,
        })
    }

    /// Returns the probability density per steradian for a world-space
    /// direction under the importance distribution.
    pub fn pdf_solid_angle(&self, direction: Vec3) -> f32 {
        let Some((u, v, sin_theta)) = self.direction_to_uv(direction) else {
            return 0.0;
        };
        let x = (u * self.width as f32).floor() as usize % self.width as usize;
        let y = ((v * self.height as f32).floor() as usize).min(self.height as usize - 1);
        self.texel_pdf_solid_angle(y * self.width as usize + x, sin_theta)
    }

    /// Normalized cumulative probability and per-texel probability pairs used
    /// by GPU renderers to mirror [`Self::sample_importance`].
    pub fn importance_entries(&self) -> impl Iterator<Item = [f32; 2]> + '_ {
        self.importance_cdf.iter().scan(0.0, |previous, &cdf| {
            let probability = (cdf - *previous).max(0.0);
            *previous = cdf;
            Some([cdf, probability])
        })
    }

    /// Maps a finite nonzero world direction to rotated `(u, v, sin(theta))`.
    fn direction_to_uv(&self, direction: Vec3) -> Option<(f32, f32, f32)> {
        let direction = direction.normalize_or_zero();
        if direction == Vec3::ZERO {
            return None;
        }
        let u = (direction.z.atan2(direction.x) / (2.0 * PI) + 0.5 + self.rotation_degrees / 360.0)
            .rem_euclid(1.0);
        let theta = direction.y.clamp(-1.0, 1.0).acos();
        Some((u, theta / PI, theta.sin()))
    }

    /// Converts one discrete texel probability to density per steradian.
    fn texel_pdf_solid_angle(&self, index: usize, sin_theta: f32) -> f32 {
        let previous_cdf = index
            .checked_sub(1)
            .map_or(0.0, |previous| self.importance_cdf[previous]);
        let probability = (self.importance_cdf[index] - previous_cdf).max(0.0);
        if probability <= 0.0 || sin_theta <= 0.0 {
            return 0.0;
        }
        probability * self.width as f32 * self.height as f32 / (2.0 * PI * PI * sin_theta)
    }

    /// Fetches a texel with horizontal wrapping and vertical clamping.
    fn texel(&self, x: i32, y: i32) -> Vec3 {
        let x = x.rem_euclid(self.width as i32) as usize;
        let y = y.clamp(0, self.height as i32 - 1) as usize;
        self.pixels[y * self.width as usize + x]
    }
}

/// Builds a normalized CDF weighted by luminance, intensity, and latitude area.
fn build_importance_distribution(
    width: u32,
    height: u32,
    pixels: &[Vec3],
    intensity: f32,
) -> Arc<[f32]> {
    let mut weights = Vec::with_capacity(pixels.len());
    let mut total = 0.0_f64;
    for (index, pixel) in pixels.iter().enumerate() {
        let y = index / width as usize;
        let theta = PI as f64 * (y as f64 + 0.5) / height as f64;
        let luminance = 0.2126 * pixel.x as f64 + 0.7152 * pixel.y as f64 + 0.0722 * pixel.z as f64;
        let weight = luminance * theta.sin() * intensity as f64;
        weights.push(weight);
        total += weight;
    }

    if total <= 0.0 || !total.is_finite() {
        return vec![0.0; pixels.len()].into();
    }

    let mut accumulated = 0.0_f64;
    let mut cdf: Vec<f32> = weights
        .iter()
        .map(|weight| {
            accumulated += weight / total;
            accumulated as f32
        })
        .collect();
    if let Some(last) = cdf.last_mut() {
        *last = 1.0;
    }
    cdf.into()
}

/// Defensively clamps an arbitrary value into the half-open unit interval.
fn unit_interval(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0 - f32::EPSILON)
    } else {
        0.0
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
    fn importance_distribution_favors_bright_texels_and_latitude() {
        let environment = EnvironmentMap::new(
            2,
            3,
            vec![
                Vec3::ONE,
                Vec3::splat(10.0),
                Vec3::ONE,
                Vec3::ONE,
                Vec3::ONE,
                Vec3::ONE,
            ],
            1.0,
            0.0,
        )
        .unwrap();
        let entries: Vec<_> = environment.importance_entries().collect();

        assert!(entries[1][1] > entries[0][1] * 9.9);
        assert!(entries[2][1] > entries[0][1]);
        assert!((entries.last().unwrap()[0] - 1.0).abs() < 1e-6);
        assert!((entries.iter().map(|entry| entry[1]).sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn importance_sample_and_direction_pdf_agree() {
        let environment = EnvironmentMap::new(
            4,
            2,
            vec![
                Vec3::splat(0.1),
                Vec3::splat(0.1),
                Vec3::splat(8.0),
                Vec3::splat(0.1),
                Vec3::splat(0.1),
                Vec3::splat(0.1),
                Vec3::splat(0.1),
                Vec3::splat(0.1),
            ],
            2.0,
            35.0,
        )
        .unwrap();

        let sample = environment.sample_importance(0.5, 0.37, 0.61).unwrap();
        assert!(sample.direction.is_normalized());
        assert!(sample.pdf_solid_angle > 0.0);
        assert!(
            (sample.pdf_solid_angle - environment.pdf_solid_angle(sample.direction)).abs() < 1e-5
        );
        assert!(sample
            .radiance
            .abs_diff_eq(environment.sample(sample.direction), 1e-6));
    }

    #[test]
    fn importance_sampling_frequencies_match_texel_probabilities() {
        let environment = EnvironmentMap::new(
            3,
            2,
            vec![
                Vec3::splat(1.0),
                Vec3::splat(4.0),
                Vec3::splat(2.0),
                Vec3::splat(3.0),
                Vec3::splat(1.0),
                Vec3::splat(5.0),
            ],
            1.0,
            27.0,
        )
        .unwrap();
        let expected: Vec<_> = environment
            .importance_entries()
            .map(|entry| entry[1])
            .collect();
        let sample_count = 10_000;
        let mut counts = vec![0_usize; expected.len()];

        for sample_index in 0..sample_count {
            let selection = (sample_index as f32 + 0.5) / sample_count as f32;
            let sample = environment.sample_importance(selection, 0.5, 0.5).unwrap();
            let (u, v, _) = environment.direction_to_uv(sample.direction).unwrap();
            let x = (u * environment.width as f32).floor() as usize;
            let y = (v * environment.height as f32).floor() as usize;
            counts[y * environment.width as usize + x] += 1;
        }

        for (count, probability) in counts.into_iter().zip(expected) {
            let observed = count as f32 / sample_count as f32;
            assert!((observed - probability).abs() <= 2.0 / sample_count as f32);
        }
    }

    #[test]
    fn black_or_disabled_environment_has_no_importance_samples() {
        let black = EnvironmentMap::new(2, 1, vec![Vec3::ZERO; 2], 1.0, 0.0).unwrap();
        let disabled = EnvironmentMap::new(1, 1, vec![Vec3::ONE], 0.0, 0.0).unwrap();

        assert!(black.sample_importance(0.5, 0.5, 0.5).is_none());
        assert_eq!(black.pdf_solid_angle(Vec3::X), 0.0);
        assert!(disabled.sample_importance(0.5, 0.5, 0.5).is_none());
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
