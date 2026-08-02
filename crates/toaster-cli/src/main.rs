mod cli;

use clap::Parser;
use cli::{Cli, Command, RenderOverrides};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use toaster_scene::RenderSettings;

const STREAM_JPEG_QUALITY: u8 = 90;

#[derive(Debug)]
struct AdaptiveSampling {
    min_samples: u32,
    max_samples: u32,
    current_samples: u32,
    frame_budget: Duration,
}

impl AdaptiveSampling {
    fn new(
        initial_samples: u32,
        min_samples: u32,
        max_samples: u32,
        target_fps: u32,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            min_samples <= max_samples,
            "--min-samples must be less than or equal to --max-samples"
        );
        anyhow::ensure!(target_fps > 0, "adaptive sampling requires a nonzero FPS");
        Ok(Self {
            min_samples,
            max_samples,
            current_samples: initial_samples.clamp(min_samples, max_samples),
            frame_budget: Duration::from_secs_f64(1.0 / target_fps as f64),
        })
    }

    fn observe(&mut self, frame_time: Duration) -> &'static str {
        let elapsed = frame_time.as_secs_f64();
        let budget = self.frame_budget.as_secs_f64();

        if elapsed > budget * 1.05 {
            if self.current_samples == self.min_samples {
                return "limited: minimum samples";
            }
            let proportional =
                (self.current_samples as f64 * budget * 0.9 / elapsed).floor() as u32;
            let lower_step = self.current_samples.div_ceil(2);
            self.current_samples = proportional
                .max(lower_step)
                .min(self.current_samples - 1)
                .max(self.min_samples);
            "reducing: over frame budget"
        } else if elapsed < budget * 0.75 {
            if self.current_samples == self.max_samples {
                return "limited: maximum samples";
            }
            let proportional = if elapsed > 0.0 {
                (self.current_samples as f64 * budget * 0.9 / elapsed).floor() as u32
            } else {
                self.max_samples
            };
            let upper_step = self.current_samples.saturating_mul(2).max(1);
            self.current_samples = proportional
                .max(self.current_samples + 1)
                .min(upper_step)
                .min(self.max_samples);
            "increasing: frame-time headroom"
        } else {
            "holding"
        }
    }
}

struct ServerFrameSink {
    publisher: toaster_server::FramePublisher,
    stop: Arc<AtomicBool>,
    target_fps: u32,
    adaptive_sampling: Option<AdaptiveSampling>,
    last_published_at: Option<Instant>,
}

impl toaster_gpu::FrameSink for ServerFrameSink {
    fn deliver(&mut self, frame: toaster_gpu::CompletedFrame<'_>) -> anyhow::Result<()> {
        let encode_started_at = Instant::now();
        let jpeg =
            toaster_gpu::image_output::encode_rgba_image_to_jpeg(frame.image, STREAM_JPEG_QUALITY)?;
        let measured_frame_time = frame.render_time + encode_started_at.elapsed();
        let (next_samples, sample_adjustment) =
            if let Some(adaptive_sampling) = &mut self.adaptive_sampling {
                let adjustment = adaptive_sampling.observe(measured_frame_time);
                (adaptive_sampling.current_samples, adjustment)
            } else {
                (frame.samples, "fixed")
            };
        let published_at = Instant::now();
        let effective_fps = self
            .last_published_at
            .and_then(|previous| {
                let elapsed = published_at.duration_since(previous).as_secs_f64();
                (elapsed > 0.0).then_some(1.0 / elapsed)
            })
            .unwrap_or(0.0);
        self.last_published_at = Some(published_at);
        self.publisher.publish_frame(
            jpeg.into(),
            toaster_server::PreviewStatus {
                frame_index: frame.index,
                animation_time_seconds: frame.time_seconds,
                render_time_ms: frame.render_time.as_secs_f64() * 1000.0,
                effective_fps,
                target_fps: self.target_fps,
                width: frame.width,
                height: frame.height,
                samples: frame.samples,
                next_samples,
                max_bounces: frame.max_bounces,
                adaptive_sampling: self.adaptive_sampling.is_some(),
                sample_adjustment: sample_adjustment.to_owned(),
            },
        );
        println!(
            "Published preview frame {} ({}x{}, {} spp -> {}) at {:.3}s.",
            frame.index, frame.width, frame.height, frame.samples, next_samples, frame.time_seconds
        );
        Ok(())
    }

    fn should_continue(&self) -> bool {
        !self.stop.load(Ordering::Relaxed)
    }

    fn samples_for_frame(&self) -> Option<u32> {
        self.adaptive_sampling
            .as_ref()
            .map(|sampling| sampling.current_samples)
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
            loop_duration,
            overrides,
            adaptive_samples,
            min_samples,
            max_samples,
        } => {
            let animation = resolve_preview_animation(fps, duration, loop_duration)?;
            let mut scene = toaster_scene::load_scene(&scene_path)?;
            apply_overrides(&mut scene.render, overrides)?;
            let adaptive_sampling = resolve_adaptive_sampling(
                adaptive_samples,
                min_samples,
                max_samples,
                scene.render.samples,
                fps,
            )?;
            println!("Scene: {}", scene_path.display());
            println!(
                "Resolution: {}x{}, samples={}, bounces={}",
                scene.render.width,
                scene.render.height,
                scene.render.samples,
                scene.render.max_bounces
            );
            match duration {
                Some(duration) => println!(
                    "Preview: fps={fps}, frames={}, duration={duration:.3}s",
                    animation
                        .frame_limit()
                        .expect("duration-based preview has a finite frame limit")
                ),
                None => println!("Preview: fps={fps}, running until Ctrl+C"),
            }
            if let Some(loop_duration) = animation.loop_duration() {
                println!("Animation loop: {loop_duration:.3}s");
            }
            if let Some(sampling) = &adaptive_sampling {
                println!(
                    "Adaptive samples: {}..={}, starting at {}",
                    sampling.min_samples, sampling.max_samples, sampling.current_samples
                );
            }
            render_stream_preview(&scene, animation, adaptive_sampling, &host, port)?;
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
    scene: &toaster_scene::Scene,
    animation: toaster_gpu::AnimationConfig,
    adaptive_sampling: Option<AdaptiveSampling>,
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
            target_fps: animation
                .fps()
                .expect("real-time preview animation has an FPS"),
            adaptive_sampling,
            last_published_at: None,
        };

        let render_result = toaster_gpu::render_gpu_animation_with_sink(
            scene,
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

fn resolve_adaptive_sampling(
    enabled: bool,
    min_samples: Option<u32>,
    max_samples: Option<u32>,
    scene_samples: u32,
    target_fps: u32,
) -> anyhow::Result<Option<AdaptiveSampling>> {
    if !enabled {
        anyhow::ensure!(
            min_samples.is_none() && max_samples.is_none(),
            "--min-samples and --max-samples require --adaptive-samples"
        );
        return Ok(None);
    }

    AdaptiveSampling::new(
        scene_samples,
        min_samples.unwrap_or(1),
        max_samples.unwrap_or(scene_samples),
        target_fps,
    )
    .map(Some)
}

fn resolve_preview_animation(
    fps: u32,
    duration: Option<f32>,
    loop_duration: Option<f32>,
) -> anyhow::Result<toaster_gpu::AnimationConfig> {
    let animation = match duration {
        Some(duration) => toaster_gpu::AnimationConfig::from_duration(fps, duration),
        None => toaster_gpu::AnimationConfig::indefinite(fps),
    }?;
    match loop_duration {
        Some(loop_duration) => animation.with_loop_duration(loop_duration),
        None => Ok(animation),
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
        let finite = resolve_preview_animation(12, Some(1.1), Some(0.5)).unwrap();
        assert_eq!(finite.frame_limit(), Some(14));
        assert_eq!(finite.fps(), Some(12));
        assert_eq!(finite.loop_duration(), Some(0.5));
        assert_eq!(finite.time_for_frame(6), 0.0);

        let indefinite = resolve_preview_animation(12, None, None).unwrap();
        assert_eq!(indefinite.frame_limit(), None);
        assert_eq!(indefinite.fps(), Some(12));
        assert_eq!(indefinite.loop_duration(), None);
    }

    #[test]
    fn adaptive_sampling_reduces_and_increases_with_frame_time() {
        let mut sampling = AdaptiveSampling::new(16, 2, 32, 10).unwrap();

        assert_eq!(
            sampling.observe(Duration::from_millis(200)),
            "reducing: over frame budget"
        );
        assert_eq!(sampling.current_samples, 8);

        assert_eq!(
            sampling.observe(Duration::from_millis(20)),
            "increasing: frame-time headroom"
        );
        assert_eq!(sampling.current_samples, 16);

        assert_eq!(sampling.observe(Duration::from_millis(90)), "holding");
        assert_eq!(sampling.current_samples, 16);
    }

    #[test]
    fn adaptive_sampling_reports_when_a_bound_prevents_adjustment() {
        let mut minimum = AdaptiveSampling::new(1, 1, 32, 10).unwrap();
        assert_eq!(
            minimum.observe(Duration::from_millis(200)),
            "limited: minimum samples"
        );

        let mut maximum = AdaptiveSampling::new(32, 1, 32, 10).unwrap();
        assert_eq!(
            maximum.observe(Duration::from_millis(20)),
            "limited: maximum samples"
        );
    }

    #[test]
    fn adaptive_sampling_resolves_defaults_and_validates_bounds() {
        let sampling = resolve_adaptive_sampling(true, None, None, 24, 12)
            .unwrap()
            .unwrap();
        assert_eq!(sampling.min_samples, 1);
        assert_eq!(sampling.max_samples, 24);
        assert_eq!(sampling.current_samples, 24);

        assert!(resolve_adaptive_sampling(true, Some(16), Some(8), 12, 12).is_err());
        assert!(resolve_adaptive_sampling(false, Some(1), None, 12, 12).is_err());
        assert!(resolve_adaptive_sampling(false, None, None, 12, 12)
            .unwrap()
            .is_none());
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
