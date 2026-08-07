//! Command-line entry point and orchestration for all Toaster workflows.

mod benchmark;
mod cli;
mod logging;

use clap::Parser;
use cli::{Cli, Command, RenderOverrides};
use std::process::ExitCode;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use toaster_scene::{RenderSettings, SceneEvaluator};

const STREAM_JPEG_QUALITY: u8 = 90;

/// Validated progressive-preview batch and target settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProgressivePreview {
    /// Samples requested for each update before final-batch clamping.
    batch_samples: u32,
    /// Total samples at convergence.
    target_samples: u32,
}

/// Feedback controller that adapts samples to a real-time frame budget.
#[derive(Debug)]
struct AdaptiveSampling {
    /// Hard lower sample limit.
    min_samples: u32,
    /// Hard upper sample limit.
    max_samples: u32,
    /// Batch size requested for the next frame.
    current_samples: u32,
    /// Duration corresponding to one target-FPS interval.
    frame_budget: Duration,
}

impl AdaptiveSampling {
    /// Creates a bounded adaptive controller with a nonzero target rate.
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

    /// Updates the next batch size from the last frame duration.
    ///
    /// Returns a stable status phrase presented by the preview server.
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

/// Converts completed frames to JPEG and publishes status to the HTTP server.
struct ServerFrameSink {
    /// Latest-frame publisher shared with all HTTP clients.
    publisher: toaster_server::FramePublisher,
    /// Cross-task cancellation flag.
    stop: Arc<AtomicBool>,
    /// Requested preview rate reported in status.
    target_fps: u32,
    /// Optional sample feedback controller.
    adaptive_sampling: Option<AdaptiveSampling>,
    /// Fixed progressive batch size when adaptation is disabled.
    fixed_batch_samples: Option<u32>,
    /// Progressive convergence target, absent for independent frames.
    progressive_target_samples: Option<u32>,
    /// Previous publication timestamp for effective-FPS calculation.
    last_published_at: Option<Instant>,
}

impl toaster_gpu::FrameSink for ServerFrameSink {
    /// JPEG-encodes, publishes, and records status for one GPU frame.
    fn deliver(&mut self, frame: toaster_gpu::CompletedFrame<'_>) -> anyhow::Result<()> {
        let encode_started_at = Instant::now();
        let jpeg =
            toaster_gpu::image_output::encode_rgba_image_to_jpeg(frame.image, STREAM_JPEG_QUALITY)?;
        let measured_frame_time = frame.render_time + encode_started_at.elapsed();
        let progress_complete = self
            .progressive_target_samples
            .is_some_and(|target| frame.accumulated_samples >= target);
        let (desired_next_samples, sample_adjustment) = if progress_complete {
            (0, "complete")
        } else if let Some(adaptive_sampling) = &mut self.adaptive_sampling {
            let adjustment = adaptive_sampling.observe(measured_frame_time);
            (adaptive_sampling.current_samples, adjustment)
        } else {
            (self.fixed_batch_samples.unwrap_or(frame.samples), "fixed")
        };
        let next_samples = self
            .progressive_target_samples
            .map_or(desired_next_samples, |target| {
                desired_next_samples.min(target.saturating_sub(frame.accumulated_samples))
            });
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
                progressive: self.progressive_target_samples.is_some(),
                accumulated_samples: frame.accumulated_samples,
                target_samples: self.progressive_target_samples,
                progress_complete,
                next_samples,
                max_bounces: frame.max_bounces,
                adaptive_sampling: self.adaptive_sampling.is_some(),
                sample_adjustment: sample_adjustment.to_owned(),
            },
        );
        if let Some(target_samples) = self.progressive_target_samples {
            tracing::debug!(
                frame.index,
                width = frame.width,
                height = frame.height,
                batch_samples = frame.samples,
                accumulated_samples = frame.accumulated_samples,
                target_samples,
                next_samples,
                animation_time_seconds = frame.time_seconds as f64,
                "published progressive preview frame"
            );
        } else {
            tracing::debug!(
                frame.index,
                width = frame.width,
                height = frame.height,
                samples = frame.samples,
                next_samples,
                animation_time_seconds = frame.time_seconds as f64,
                "published preview frame"
            );
        }
        Ok(())
    }

    /// Stops after the HTTP task exits or Ctrl+C sets the shared flag.
    fn should_continue(&self) -> bool {
        !self.stop.load(Ordering::Relaxed)
    }

    /// Selects the next adaptive or fixed progressive sample count.
    fn samples_for_frame(&self) -> Option<u32> {
        self.adaptive_sampling
            .as_ref()
            .map(|sampling| sampling.current_samples)
            .or(self.fixed_batch_samples)
    }
}

/// Parses arguments, initializes logging, and maps success to a process exit code.
fn main() -> ExitCode {
    let Cli {
        log_level,
        log_format,
        command,
    } = Cli::parse();
    if let Err(error) = logging::init(log_level, log_format) {
        eprintln!("failed to initialize logging: {error:#}");
        return ExitCode::FAILURE;
    }

    match run(command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "command failed");
            ExitCode::FAILURE
        }
    }
}

/// Executes one parsed CLI command.
fn run(command: Command) -> anyhow::Result<()> {
    match command {
        Command::CpuRender {
            scene_path,
            out,
            overrides,
        } => {
            let mut scene = toaster_scene::load_scene(&scene_path)?;
            apply_overrides(&mut scene.render, overrides)?;
            tracing::info!(
                scene = %scene_path.display(),
                output = %out.display(),
                width = scene.render.width,
                height = scene.render.height,
                samples = scene.render.samples,
                max_bounces = scene.render.max_bounces,
                "starting CPU render"
            );

            let start = Instant::now();
            let image = toaster_cpu::render(&scene);
            image.save_png(&out)?;
            tracing::info!(
                output = %out.display(),
                duration_ms = start.elapsed().as_secs_f64() * 1000.0,
                "completed CPU render"
            );
        }
        Command::GpuRender {
            scene_path,
            out,
            video,
            fps,
            duration,
            frames,
        } => {
            let animation = resolve_animation(fps, duration, frames)?;
            let scene = toaster_scene::load_scene(&scene_path)?;
            let mut evaluator = toaster_physics::PhysicsSceneEvaluator::new(scene)?;
            match animation.fps() {
                Some(fps) => tracing::info!(
                    scene = %scene_path.display(),
                    fps,
                    frames = animation.frame_limit().unwrap_or_default(),
                    "starting animated GPU render"
                ),
                None => tracing::info!(
                    scene = %scene_path.display(),
                    "starting single-frame GPU render"
                ),
            }

            match (out, video) {
                (Some(out), None) => {
                    tracing::info!(output = %out.display(), "selected PNG output");
                    pollster::block_on(toaster_gpu::render_evaluated_gpu_animation(
                        &mut evaluator,
                        &out,
                        animation,
                    ))?;
                }
                (None, Some(video)) => {
                    tracing::info!(output = %video.display(), "selected MP4 output");
                    pollster::block_on(toaster_gpu::render_evaluated_gpu_video(
                        &mut evaluator,
                        &video,
                        animation,
                    ))?;
                    tracing::info!(output = %video.display(), "wrote MP4 video");
                }
                _ => unreachable!("clap requires exactly one GPU output"),
            }
        }
        Command::StreamPreview {
            scene_path,
            host,
            port,
            fps,
            duration,
            loop_duration,
            overrides,
            progressive,
            batch_samples,
            target_samples,
            adaptive_samples,
            min_samples,
            max_samples,
        } => {
            let animation = resolve_preview_animation(fps, duration, loop_duration)?;
            let mut scene = toaster_scene::load_scene(&scene_path)?;
            let progressive = resolve_progressive_preview(
                progressive,
                batch_samples,
                target_samples,
                overrides.samples,
                loop_duration,
                &scene,
            )?;
            apply_overrides(&mut scene.render, overrides)?;
            let (initial_samples, default_max_samples) = progressive.map_or(
                (scene.render.samples, scene.render.samples),
                |progressive| (progressive.batch_samples, progressive.target_samples),
            );
            let adaptive_sampling = resolve_adaptive_sampling(
                adaptive_samples,
                min_samples,
                max_samples,
                initial_samples,
                default_max_samples,
                progressive.map(|config| config.target_samples),
                fps,
            )?;
            tracing::info!(
                scene = %scene_path.display(),
                width = scene.render.width,
                height = scene.render.height,
                samples = scene.render.samples,
                max_bounces = scene.render.max_bounces,
                fps,
                "starting stream preview"
            );
            match duration {
                Some(duration) => tracing::info!(
                    frames = animation
                        .frame_limit()
                        .expect("duration-based preview has a finite frame limit"),
                    duration_seconds = duration as f64,
                    "configured finite preview"
                ),
                None => tracing::info!("preview will run until Ctrl+C"),
            }
            if let Some(loop_duration) = animation.loop_duration() {
                tracing::info!(
                    loop_duration_seconds = loop_duration as f64,
                    "configured animation loop"
                );
            }
            if let Some(progressive) = progressive {
                tracing::info!(
                    batch_samples = progressive.batch_samples,
                    target_samples = progressive.target_samples,
                    "configured progressive preview"
                );
            }
            if let Some(sampling) = &adaptive_sampling {
                tracing::info!(
                    min_samples = sampling.min_samples,
                    max_samples = sampling.max_samples,
                    initial_samples = sampling.current_samples,
                    "configured adaptive sampling"
                );
            }
            let mut evaluator = toaster_physics::PhysicsSceneEvaluator::new(scene)?;
            render_stream_preview(
                &mut evaluator,
                animation,
                adaptive_sampling,
                progressive,
                &host,
                port,
            )?;
        }
        Command::Benchmark {
            scene_path,
            warmup,
            runs,
            out,
            image_out,
            compare,
            max_regression_percent,
            overrides,
        } => {
            let baseline = compare.as_deref().map(benchmark::read_report).transpose()?;
            let mut scene = toaster_scene::load_scene(&scene_path)?;
            apply_overrides(&mut scene.render, overrides)?;
            let config = toaster_gpu::GpuBenchmarkConfig::new(warmup, runs)?;

            tracing::info!(
                scene = %scene_path.display(),
                width = scene.render.width,
                height = scene.render.height,
                samples = scene.render.samples,
                max_bounces = scene.render.max_bounces,
                warmup_frames = warmup,
                measured_frames = runs,
                "starting GPU benchmark"
            );
            let mut evaluator = toaster_physics::PhysicsSceneEvaluator::new(scene.clone())?;
            let result =
                pollster::block_on(toaster_gpu::benchmark_gpu_evaluator(&mut evaluator, config))?;

            if let Some(image_path) = image_out.as_deref() {
                benchmark::save_reference_image(&result, image_path)?;
                tracing::info!(path = %image_path.display(), "wrote benchmark reference image");
            }
            let report = benchmark::build_report(
                &scene_path,
                &scene,
                config,
                &result,
                image_out.as_deref(),
            )?;
            benchmark::write_report(&report, &out)?;
            tracing::info!(
                path = %out.display(),
                setup_ms = report.setup_ms,
                median_animation_evaluation_ms = report.summary.animation_evaluation_ms.median,
                median_physics_evaluation_ms = report.summary.physics_evaluation_ms.median,
                median_geometry_update_ms = report.summary.geometry_update_ms.median,
                median_light_rebuild_ms = report.summary.light_rebuild_ms.median,
                median_bvh_rebuild_ms = report.summary.bvh_rebuild_ms.median,
                median_gpu_upload_ms = report.summary.gpu_upload_ms.median,
                median_total_ms = report.summary.total_ms.median,
                p95_total_ms = report.summary.total_ms.p95,
                image_sha256 = %report.image.rgba_sha256,
                "wrote GPU benchmark report"
            );

            if let Some(baseline) = baseline {
                let comparison =
                    benchmark::compare_reports(&baseline, &report, max_regression_percent)?;
                let deltas = comparison.deltas;
                tracing::info!(
                    setup_percent = ?deltas.setup_percent,
                    scene_update_upload_percent = ?deltas.scene_update_upload_percent,
                    dispatch_wait_percent = ?deltas.dispatch_wait_percent,
                    readback_percent = ?deltas.readback_percent,
                    conversion_percent = ?deltas.conversion_percent,
                    total_percent = ?deltas.total_percent,
                    "compared GPU benchmark with baseline"
                );
                if !comparison.matching_gpu {
                    tracing::warn!(
                        "benchmark GPU identity differs from the baseline; timing deltas are informational"
                    );
                } else if !comparison.matching_driver {
                    tracing::warn!(
                        "benchmark GPU driver differs from the baseline; timing deltas may include driver changes"
                    );
                }
                if !comparison.matching_image_checksum {
                    tracing::warn!(
                        "benchmark image checksum differs from the baseline; exact pixels may vary across drivers"
                    );
                }
                anyhow::ensure!(
                    !comparison.exceeds_regression_limit,
                    "benchmark median total frame time exceeded the allowed regression of {}%",
                    max_regression_percent.expect("comparison only exceeds an explicit limit")
                );
            }
        }
        Command::Server { host, port } => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(toaster_server::serve(&host, port))?;
        }
        Command::Info => {
            println!("Toaster {}", env!("CARGO_PKG_VERSION"));
            println!(
                "Modules: core, scene, physics, CPU renderer, GPU renderer, BVH, assets, server"
            );
        }
    }
    Ok(())
}

/// Runs the preview server, signal task, and synchronous sink-driven GPU loop.
///
/// The listener is bound before GPU setup. Both background tasks are always
/// stopped and awaited (or aborted and joined) before this function returns.
fn render_stream_preview(
    evaluator: &mut dyn SceneEvaluator,
    animation: toaster_gpu::AnimationConfig,
    adaptive_sampling: Option<AdaptiveSampling>,
    progressive: Option<ProgressivePreview>,
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
                    tracing::info!("stopping preview after the current frame");
                    signal_stop.store(true, Ordering::Relaxed);
                }
                Err(error) => tracing::warn!(%error, "failed to listen for Ctrl+C"),
            }
        });

        let mut sink = ServerFrameSink {
            publisher,
            stop: stop.clone(),
            target_fps: animation
                .fps()
                .expect("real-time preview animation has an FPS"),
            adaptive_sampling,
            fixed_batch_samples: progressive.map(|config| config.batch_samples),
            progressive_target_samples: progressive.map(|config| config.target_samples),
            last_published_at: None,
        };

        let render_result = match progressive {
            Some(progressive) => {
                toaster_gpu::render_gpu_progressive_with_sink(
                    evaluator.source_scene(),
                    animation,
                    toaster_gpu::FramePacing::RealTime,
                    toaster_gpu::ProgressiveRenderConfig::new(progressive.target_samples)?,
                    &mut sink,
                )
                .await
            }
            None => {
                toaster_gpu::render_evaluated_gpu_animation_with_sink(
                    evaluator,
                    animation,
                    toaster_gpu::FramePacing::RealTime,
                    &mut sink,
                )
                .await
            }
        };

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

/// Validates progressive-only flags and derives defaults from the scene.
fn resolve_progressive_preview(
    enabled: bool,
    batch_samples: Option<u32>,
    target_samples: Option<u32>,
    sample_override: Option<u32>,
    loop_duration: Option<f32>,
    scene: &toaster_scene::Scene,
) -> anyhow::Result<Option<ProgressivePreview>> {
    if !enabled {
        anyhow::ensure!(
            batch_samples.is_none() && target_samples.is_none(),
            "--batch-samples and --target-samples require --progressive"
        );
        return Ok(None);
    }

    anyhow::ensure!(
        sample_override.is_none(),
        "--samples cannot be used with --progressive; use --batch-samples and --target-samples"
    );
    anyhow::ensure!(
        loop_duration.is_none(),
        "--loop-duration cannot be used with --progressive"
    );
    anyhow::ensure!(
        !scene.physics.is_some_and(|physics| physics.enabled),
        "--progressive does not support physics-enabled scenes"
    );
    anyhow::ensure!(
        scene.animation.tracks.is_empty(),
        "--progressive requires a scene without animation tracks"
    );

    let config = ProgressivePreview {
        batch_samples: batch_samples.unwrap_or(1),
        target_samples: target_samples.unwrap_or(scene.render.samples),
    };
    anyhow::ensure!(
        config.batch_samples > 0 && config.target_samples > 0,
        "progressive sample counts must be greater than zero"
    );
    anyhow::ensure!(
        config.batch_samples <= config.target_samples,
        "--batch-samples must be less than or equal to --target-samples"
    );
    Ok(Some(config))
}

/// Validates adaptive sampling bounds and constructs its controller when enabled.
fn resolve_adaptive_sampling(
    enabled: bool,
    min_samples: Option<u32>,
    max_samples: Option<u32>,
    initial_samples: u32,
    default_max_samples: u32,
    sample_ceiling: Option<u32>,
    target_fps: u32,
) -> anyhow::Result<Option<AdaptiveSampling>> {
    if !enabled {
        anyhow::ensure!(
            min_samples.is_none() && max_samples.is_none(),
            "--min-samples and --max-samples require --adaptive-samples"
        );
        return Ok(None);
    }

    if let Some(sample_ceiling) = sample_ceiling {
        let resolved_min = min_samples.unwrap_or(1);
        let resolved_max = max_samples.unwrap_or(default_max_samples);
        anyhow::ensure!(
            resolved_min <= sample_ceiling && resolved_max <= sample_ceiling,
            "adaptive sample bounds cannot exceed the progressive target"
        );
        anyhow::ensure!(
            resolved_min <= initial_samples && initial_samples <= resolved_max,
            "progressive batch samples must fall within the adaptive sample bounds"
        );
    }

    AdaptiveSampling::new(
        initial_samples,
        min_samples.unwrap_or(1),
        max_samples.unwrap_or(default_max_samples),
        target_fps,
    )
    .map(Some)
}

/// Builds the finite or indefinite real-time preview schedule.
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

/// Resolves the mutually exclusive single-frame, duration, and frame-count forms.
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

/// Applies CLI render overrides and validates the resolved nonzero settings.
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
        let sampling = resolve_adaptive_sampling(true, None, None, 24, 24, None, 12)
            .unwrap()
            .unwrap();
        assert_eq!(sampling.min_samples, 1);
        assert_eq!(sampling.max_samples, 24);
        assert_eq!(sampling.current_samples, 24);

        assert!(resolve_adaptive_sampling(true, Some(16), Some(8), 12, 12, None, 12).is_err());
        assert!(resolve_adaptive_sampling(false, Some(1), None, 12, 12, None, 12).is_err());
        assert!(
            resolve_adaptive_sampling(false, None, None, 12, 12, None, 12)
                .unwrap()
                .is_none()
        );
        assert!(resolve_adaptive_sampling(true, None, Some(17), 2, 16, Some(16), 12).is_err());
    }

    #[test]
    fn resolves_progressive_defaults_and_rejects_incompatible_scenes() {
        let static_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/010_environment_map.json");
        let static_scene = toaster_scene::load_scene(static_path).unwrap();
        let defaults =
            resolve_progressive_preview(true, None, None, None, None, &static_scene).unwrap();
        assert_eq!(
            defaults,
            Some(ProgressivePreview {
                batch_samples: 1,
                target_samples: static_scene.render.samples,
            })
        );
        assert_eq!(
            resolve_progressive_preview(true, Some(4), Some(64), None, None, &static_scene,)
                .unwrap(),
            Some(ProgressivePreview {
                batch_samples: 4,
                target_samples: 64,
            })
        );

        assert!(
            resolve_progressive_preview(true, Some(65), Some(64), None, None, &static_scene,)
                .is_err()
        );
        assert!(
            resolve_progressive_preview(true, None, None, Some(2), None, &static_scene).is_err()
        );

        let animated_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/006_rotating_cube.json");
        let animated_scene = toaster_scene::load_scene(animated_path).unwrap();
        assert!(
            resolve_progressive_preview(true, None, None, None, None, &animated_scene).is_err()
        );

        let physics_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/011_physics_rigid_bodies.json");
        let mut physics_scene = toaster_scene::load_scene(physics_path).unwrap();
        let error =
            resolve_progressive_preview(true, None, None, None, None, &physics_scene).unwrap_err();
        assert!(error.to_string().contains("physics-enabled"));

        let kinematic_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenes/012_physics_kinematic_platform.json");
        let kinematic_scene = toaster_scene::load_scene(kinematic_path).unwrap();
        let error = resolve_progressive_preview(true, None, None, None, None, &kinematic_scene)
            .unwrap_err();
        assert!(error.to_string().contains("physics-enabled"));

        physics_scene.animation.tracks.clear();
        physics_scene.physics.as_mut().unwrap().enabled = false;
        assert!(resolve_progressive_preview(true, None, None, None, None, &physics_scene).is_ok());
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
