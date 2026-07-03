use glam::Vec3;

pub fn linear_to_rgb8(color: Vec3) -> [u8; 3] {
    let gamma_corrected = Vec3::new(
        color.x.max(0.0).sqrt(),
        color.y.max(0.0).sqrt(),
        color.z.max(0.0).sqrt(),
    )
    .clamp(Vec3::ZERO, Vec3::splat(0.999));
    [
        (gamma_corrected.x * 256.0) as u8,
        (gamma_corrected.y * 256.0) as u8,
        (gamma_corrected.z * 256.0) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_and_clamps_linear_color() {
        assert_eq!(linear_to_rgb8(Vec3::new(-1.0, 0.25, 2.0)), [0, 128, 255]);
    }
}
