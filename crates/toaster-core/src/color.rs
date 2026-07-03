use glam::Vec3;

pub fn linear_to_rgb8(color: Vec3) -> [u8; 3] {
    let color = color.clamp(Vec3::ZERO, Vec3::ONE);
    [
        (color.x.sqrt() * 255.999) as u8,
        (color.y.sqrt() * 255.999) as u8,
        (color.z.sqrt() * 255.999) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_and_clamps_linear_color() {
        assert_eq!(linear_to_rgb8(Vec3::new(-1.0, 0.25, 2.0)), [0, 127, 255]);
    }
}
