mod cli;

use clap::Parser;
use cli::{Cli, Command, RenderOverrides};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;
use toaster_scene::RenderSettings;

const STREAM_JPEG_QUALITY: u8 = 90;

struct ServerFrameSink {
    publisher: toaster_server::FramePublisher,
    stop: Arc<AtomicBool>,
}

impl toaster_gpu::FrameSink for ServerFrameSink {
    fn deliver(&mut self, frame: toaster_gpu::CompletedFrame<'_>) -> anyhow::Result<()> {
        let jpeg =
            toaster_gpu::image_output::encode_rgba_image_to_jpeg(frame.image, STREAM_JPEG_QUALITY)?;
        self.publisher.publish_jpeg(jpeg.into());
        println!(
            "Published preview frame {} ({}x{}) at {:.3}s.",
            frame.index, frame.width, frame.height, frame.time_seconds
        );
        Ok(())
    }

    fn should_continue(&self) -> bool {
        !self.stop.load(Ordering::Relaxed)
    }
}

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
            frames,
        } => {
            println!("Scene: {}", scene_path.display());
            println!("Output: {}", out.display());

            let animation = resolve_animation(fps, duration, frames)?;
            match animation.fps() {
                Some(fps) => println!(
                    "Animation: fps={}, frames={}",
                    fps,
                    animation.frame_limit().unwrap_or_default()
                ),
                None => println!("Animation: single frame at time 0"),
            }

            pollster::block_on(toaster_gpu::render_scene_gpu_animation(
                &scene_path,
                &out,
                animation,
            ))?;
        }
        Command::StreamPreview {
            scene_path,
            host,
            port,
            fps,
            duration,
        } => {
            let animation = resolve_preview_animation(fps, duration)?;
            println!("Scene: {}", scene_path.display());
            match duration {
                Some(duration) => println!(
                    "Preview: fps={fps}, frames={}, duration={duration:.3}s",
                    animation
                        .frame_limit()
                        .expect("duration-based preview has a finite frame limit")
                ),
                None => println!("Preview: fps={fps}, running until Ctrl+C"),
            }
            render_stream_preview(&scene_path, animation, &host, port)?;
        }
        Command::Server { host, port } => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(toaster_server::serve(&host, port))?;
        }
        Command::Info => {
            println!("Toaster {}", env!("CARGO_PKG_VERSION"));
            println!("Modules: core, scene, CPU renderer, GPU renderer, BVH, assets, server");
        }
    }
    Ok(())
}

fn render_stream_preview(
    scene_path: &std::path::Path,
    animation: toaster_gpu::AnimationConfig,
    host: &str,
    port: u16,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let publisher = toaster_server::FramePublisher::new();
        let listener = toaster_server::bind(host, port).await?;
        let stop = Arc::new(AtomicBool::new(false));

        let server_publisher = publisher.clone();
        let server_stop = stop.clone();
        let server = tokio::spawn(async move {
            let result = toaster_server::serve_listener(listener, server_publisher).await;
            server_stop.store(true, Ordering::Relaxed);
            result
        });

        let signal_stop = stop.clone();
        let signal = tokio::spawn(async move {
            match tokio::signal::ctrl_c().await {
                Ok(()) => {
                    println!("Stopping preview after the current frame.");
                    signal_stop.store(true, Ordering::Relaxed);
                }
                Err(error) => eprintln!("Failed to listen for Ctrl+C: {error}"),
            }
        });

        let mut sink = ServerFrameSink {
            publisher,
            stop: stop.clone(),
        };

        let render_result = toaster_gpu::render_scene_gpu_animation_with_sink(
            scene_path,
            animation,
            toaster_gpu::FramePacing::RealTime,
            &mut sink,
        )
        .await;

        stop.store(true, Ordering::Relaxed);
        signal.abort();
        let _ = signal.await;

        let server_result = if server.is_finished() {
            match server.await {
                Ok(result) => result,
                Err(error) => Err(error.into()),
            }
        } else {
            server.abort();
            let _ = server.await;
            Ok(())
        };

        render_result?;
        server_result
    })
}

fn resolve_preview_animation(
    fps: u32,
    duration: Option<f32>,
) -> anyhow::Result<toaster_gpu::AnimationConfig> {
    match duration {
        Some(duration) => toaster_gpu::AnimationConfig::from_duration(fps, duration),
        None => toaster_gpu::AnimationConfig::indefinite(fps),
    }
}

fn resolve_animation(
    fps: Option<u32>,
    duration: Option<f32>,
    frames: Option<u32>,
) -> anyhow::Result<toaster_gpu::AnimationConfig> {
    match (fps, duration, frames) {
        (None, None, None) => Ok(toaster_gpu::AnimationConfig::single_frame()),
        (Some(fps), Some(duration), None) => {
            toaster_gpu::AnimationConfig::from_duration(fps, duration)
        }
        (Some(fps), None, Some(frames)) => {
            toaster_gpu::AnimationConfig::from_frame_count(fps, frames)
        }
        (None, _, _) => anyhow::bail!("animated renders require --fps"),
        (Some(_), None, None) => {
            anyhow::bail!("--fps requires exactly one of --duration or --frames")
        }
        (Some(_), Some(_), Some(_)) => {
            anyhow::bail!("--duration and --frames are mutually exclusive")
        }
    }
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

    #[test]
    fn resolves_explicit_animation_timing() {
        let duration = resolve_animation(Some(24), Some(1.1), None).unwrap();
        assert_eq!(duration.frame_limit(), Some(27));
        assert_eq!(duration.fps(), Some(24));

        let frames = resolve_animation(Some(30), None, Some(12)).unwrap();
        assert_eq!(frames.frame_limit(), Some(12));
        assert_eq!(frames.fps(), Some(30));
    }

    #[test]
    fn resolves_finite_and_indefinite_preview_timing() {
        let finite = resolve_preview_animation(12, Some(1.1)).unwrap();
        assert_eq!(finite.frame_limit(), Some(14));
        assert_eq!(finite.fps(), Some(12));

        let indefinite = resolve_preview_animation(12, None).unwrap();
        assert_eq!(indefinite.frame_limit(), None);
        assert_eq!(indefinite.fps(), Some(12));
    }

    #[test]
    fn rejects_incomplete_or_conflicting_animation_timing() {
        for timing in [
            (Some(24), None, None),
            (None, Some(1.0), None),
            (None, None, Some(24)),
            (Some(24), Some(1.0), Some(24)),
            (Some(0), Some(1.0), None),
            (Some(24), Some(f32::NAN), None),
            (Some(24), None, Some(0)),
        ] {
            assert!(resolve_animation(timing.0, timing.1, timing.2).is_err());
        }
    }
}
