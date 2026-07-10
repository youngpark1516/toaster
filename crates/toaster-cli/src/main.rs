mod cli;

use clap::Parser;
use cli::{Cli, Command, RenderOverrides};
use std::time::Instant;
use toaster_scene::RenderSettings;

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::CpuRender {
            scene_path,
            out,
            overrides,
        } => {
            let mut scene = toaster_scene::load_scene(&scene_path)?;
            apply_overrides(&mut scene.render, overrides)?;
            println!("Scene: {}", scene_path.display());
            println!("Output: {}", out.display());
            println!("Resolution: {}x{}", scene.render.width, scene.render.height);
            println!("Samples: {}", scene.render.samples);
            println!("Max bounces: {}", scene.render.max_bounces);

            let start = Instant::now();
            let image = toaster_cpu::render(&scene);
            image.save_png(&out)?;
            println!("Rendered in {:.2?}", start.elapsed());
        }
        Command::GpuRender {
            scene_path,
            out,
            fps,
            duration,
        } => {
            println!("Scene: {}", scene_path.display());
            println!("Output: {}", out.display());

            let animation = match (fps, duration) {
                (None, None) => toaster_gpu::AnimationConfig::single_frame(),
                _ => {
                    let fps = fps.unwrap_or(24);
                    let duration_seconds = duration.unwrap_or(1.0);

                    anyhow::ensure!(fps > 0, "fps must be greater than zero");
                    anyhow::ensure!(
                        duration_seconds > 0.0,
                        "duration must be greater than zero"
                    );

                    toaster_gpu::AnimationConfig {
                        fps,
                        duration_seconds,
                    }
                }
            };

            println!(
                "Animation: fps={}, duration={:.3}s, frames={}",
                animation.fps,
                animation.duration_seconds,
                animation.frame_count()
            );

            pollster::block_on(toaster_gpu::render_scene_gpu_animation(
                &scene_path,
                &out,
                animation,
            ))?;
        }
        Command::Server { host, port } => {
            println!("Server placeholder: http://{host}:{port}");
        }
        Command::Info => {
            println!("Toaster {}", env!("CARGO_PKG_VERSION"));
            println!("Modules: core, scene, CPU renderer, GPU renderer, BVH, assets, server");
        }
    }
    Ok(())
}

fn apply_overrides(
    settings: &mut RenderSettings,
    overrides: RenderOverrides,
) -> anyhow::Result<()> {
    if let Some(samples) = overrides.samples {
        settings.samples = samples;
    }
    if let Some(width) = overrides.width {
        settings.width = width;
    }
    if let Some(height) = overrides.height {
        settings.height = height;
    }
    if let Some(max_bounces) = overrides.max_bounces {
        settings.max_bounces = max_bounces;
    }

    anyhow::ensure!(settings.width > 0, "render width must be greater than zero");
    anyhow::ensure!(
        settings.height > 0,
        "render height must be greater than zero"
    );
    anyhow::ensure!(
        settings.samples > 0,
        "render samples must be greater than zero"
    );
    anyhow::ensure!(
        settings.max_bounces > 0,
        "render max bounces must be greater than zero"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use toaster_scene::Background;

    fn settings() -> RenderSettings {
        RenderSettings {
            width: 800,
            height: 450,
            samples: 16,
            max_bounces: 8,
            background: Background::Sky,
        }
    }

    #[test]
    fn applies_only_provided_overrides() {
        let mut settings = settings();
        apply_overrides(
            &mut settings,
            RenderOverrides {
                samples: Some(32),
                width: Some(400),
                ..RenderOverrides::default()
            },
        )
        .unwrap();
        assert_eq!(settings.width, 400);
        assert_eq!(settings.height, 450);
        assert_eq!(settings.samples, 32);
        assert_eq!(settings.max_bounces, 8);
    }

    #[test]
    fn rejects_zero_resolved_values() {
        for overrides in [
            RenderOverrides {
                width: Some(0),
                ..RenderOverrides::default()
            },
            RenderOverrides {
                height: Some(0),
                ..RenderOverrides::default()
            },
            RenderOverrides {
                samples: Some(0),
                ..RenderOverrides::default()
            },
            RenderOverrides {
                max_bounces: Some(0),
                ..RenderOverrides::default()
            },
        ] {
            assert!(apply_overrides(&mut settings(), overrides).is_err());
        }
    }
}
