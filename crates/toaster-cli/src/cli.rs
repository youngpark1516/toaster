use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "toaster",
    version,
    about = "Headless path tracer and scene sandbox"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    CpuRender {
        scene_path: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[command(flatten)]
        overrides: RenderOverrides,
    },
    GpuRender {
        scene_path: PathBuf,

        /// PNG output path. Animated renders write a numbered PNG sequence.
        #[arg(long, required_unless_present = "video", conflicts_with = "video")]
        out: Option<PathBuf>,

        /// Encode completed frames directly into an H.264 MP4 using FFmpeg.
        #[arg(
            long,
            value_name = "MP4",
            required_unless_present = "out",
            conflicts_with = "out",
            requires = "fps"
        )]
        video: Option<PathBuf>,

        /// Frames rendered per second. Required for animated output.
        #[arg(long)]
        fps: Option<u32>,

        /// Animation duration in seconds; conflicts with --frames.
        #[arg(long, requires = "fps", conflicts_with = "frames")]
        duration: Option<f32>,

        /// Exact animation frame count; conflicts with --duration.
        #[arg(long, requires = "fps", conflicts_with = "duration")]
        frames: Option<u32>,
    },
    StreamPreview {
        scene_path: PathBuf,

        /// Address used by the preview server.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port used by the preview server.
        #[arg(long, default_value_t = 7878)]
        port: u16,

        /// Target completed frames per second.
        #[arg(
            long,
            default_value_t = 12,
            value_parser = clap::value_parser!(u32).range(1..)
        )]
        fps: u32,

        /// Optional preview duration in seconds. Omit to run until Ctrl+C.
        #[arg(long, value_parser = parse_positive_f32)]
        duration: Option<f32>,

        /// Wrap animation time at this interval while frame indices continue increasing.
        #[arg(long, value_parser = parse_positive_f32, conflicts_with = "progressive")]
        loop_duration: Option<f32>,

        /// Accumulate batches for a static scene until a target sample count is reached.
        #[arg(long, conflicts_with = "samples")]
        progressive: bool,

        /// Samples added by each progressive update. Defaults to 1.
        #[arg(
            long,
            requires = "progressive",
            value_parser = clap::value_parser!(u32).range(1..)
        )]
        batch_samples: Option<u32>,

        /// Total samples accumulated by a progressive preview. Defaults to the scene setting.
        #[arg(
            long,
            requires = "progressive",
            value_parser = clap::value_parser!(u32).range(1..)
        )]
        target_samples: Option<u32>,

        #[command(flatten)]
        overrides: RenderOverrides,

        /// Adjust samples per frame to stay near the target FPS.
        #[arg(long)]
        adaptive_samples: bool,

        /// Lowest sample count used by adaptive preview.
        #[arg(
            long,
            requires = "adaptive_samples",
            value_parser = clap::value_parser!(u32).range(1..)
        )]
        min_samples: Option<u32>,

        /// Highest sample count used by adaptive preview.
        #[arg(
            long,
            requires = "adaptive_samples",
            value_parser = clap::value_parser!(u32).range(1..)
        )]
        max_samples: Option<u32>,
    },
    Server {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 7878)]
        port: u16,
    },
    Info,
}

fn parse_positive_f32(value: &str) -> Result<f32, String> {
    let parsed = value
        .parse::<f32>()
        .map_err(|_| format!("invalid floating-point value: {value}"))?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err("value must be finite and greater than zero".to_owned());
    }
    Ok(parsed)
}

#[derive(Clone, Copy, Debug, Default, Args)]
pub struct RenderOverrides {
    /// Override the scene's samples per pixel.
    #[arg(long)]
    pub samples: Option<u32>,
    /// Override the scene's output width.
    #[arg(long)]
    pub width: Option<u32>,
    /// Override the scene's output height.
    #[arg(long)]
    pub height: Option<u32>,
    /// Override the scene's maximum path depth.
    #[arg(long)]
    pub max_bounces: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu_render_overrides() {
        let cli = Cli::try_parse_from([
            "toaster",
            "cpu-render",
            "scene.json",
            "--out",
            "image.png",
            "--samples",
            "512",
            "--width",
            "800",
            "--height",
            "600",
            "--max-bounces",
            "12",
        ])
        .unwrap();

        let Command::CpuRender { overrides, .. } = cli.command else {
            panic!("expected cpu-render command");
        };
        assert_eq!(overrides.samples, Some(512));
        assert_eq!(overrides.width, Some(800));
        assert_eq!(overrides.height, Some(600));
        assert_eq!(overrides.max_bounces, Some(12));
    }

    #[test]
    fn parses_both_gpu_timing_shapes() {
        for args in [
            vec![
                "toaster",
                "gpu-render",
                "scene.json",
                "--out",
                "image.png",
                "--fps",
                "24",
                "--duration",
                "2",
            ],
            vec![
                "toaster",
                "gpu-render",
                "scene.json",
                "--out",
                "image.png",
                "--fps",
                "30",
                "--frames",
                "45",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
    }

    #[test]
    fn parses_video_output_and_rejects_conflicting_outputs() {
        let cli = Cli::try_parse_from([
            "toaster",
            "gpu-render",
            "scene.json",
            "--video",
            "animation.mp4",
            "--fps",
            "24",
            "--duration",
            "2",
        ])
        .unwrap();
        let Command::GpuRender { out, video, .. } = cli.command else {
            panic!("expected gpu-render command");
        };
        assert!(out.is_none());
        assert_eq!(video, Some(PathBuf::from("animation.mp4")));

        assert!(Cli::try_parse_from([
            "toaster",
            "gpu-render",
            "scene.json",
            "--out",
            "frames.png",
            "--video",
            "animation.mp4",
            "--fps",
            "24",
            "--frames",
            "2",
        ])
        .is_err());
        assert!(Cli::try_parse_from(["toaster", "gpu-render", "scene.json"]).is_err());
        assert!(Cli::try_parse_from([
            "toaster",
            "gpu-render",
            "scene.json",
            "--video",
            "animation.mp4"
        ])
        .is_err());
    }

    #[test]
    fn parses_stream_preview_defaults_and_explicit_options() {
        let defaults = Cli::try_parse_from(["toaster", "stream-preview", "scene.json"]).unwrap();
        let Command::StreamPreview {
            host,
            port,
            fps,
            duration,
            loop_duration,
            overrides,
            adaptive_samples,
            min_samples,
            max_samples,
            ..
        } = defaults.command
        else {
            panic!("expected stream-preview command");
        };
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 7878);
        assert_eq!(fps, 12);
        assert_eq!(duration, None);
        assert_eq!(loop_duration, None);
        assert_eq!(overrides.width, None);
        assert!(!adaptive_samples);
        assert_eq!(min_samples, None);
        assert_eq!(max_samples, None);

        let cli = Cli::try_parse_from([
            "toaster",
            "stream-preview",
            "scene.json",
            "--host",
            "0.0.0.0",
            "--port",
            "9000",
            "--fps",
            "24",
            "--duration",
            "1.5",
            "--loop-duration",
            "2",
            "--width",
            "640",
            "--height",
            "360",
            "--samples",
            "8",
            "--max-bounces",
            "4",
            "--adaptive-samples",
            "--min-samples",
            "2",
            "--max-samples",
            "32",
        ])
        .unwrap();

        let Command::StreamPreview {
            host,
            port,
            fps,
            duration,
            loop_duration,
            overrides,
            adaptive_samples,
            min_samples,
            max_samples,
            ..
        } = cli.command
        else {
            panic!("expected stream-preview command");
        };
        assert_eq!(host, "0.0.0.0");
        assert_eq!(port, 9000);
        assert_eq!(fps, 24);
        assert_eq!(duration, Some(1.5));
        assert_eq!(loop_duration, Some(2.0));
        assert_eq!(overrides.width, Some(640));
        assert_eq!(overrides.height, Some(360));
        assert_eq!(overrides.samples, Some(8));
        assert_eq!(overrides.max_bounces, Some(4));
        assert!(adaptive_samples);
        assert_eq!(min_samples, Some(2));
        assert_eq!(max_samples, Some(32));
    }

    #[test]
    fn rejects_invalid_stream_preview_timing() {
        for args in [
            vec!["toaster", "stream-preview", "scene.json", "--fps", "0"],
            vec!["toaster", "stream-preview", "scene.json", "--duration", "0"],
            vec![
                "toaster",
                "stream-preview",
                "scene.json",
                "--duration",
                "NaN",
            ],
            vec![
                "toaster",
                "stream-preview",
                "scene.json",
                "--loop-duration",
                "0",
            ],
            vec![
                "toaster",
                "stream-preview",
                "scene.json",
                "--adaptive-samples",
                "--min-samples",
                "0",
            ],
            vec![
                "toaster",
                "stream-preview",
                "scene.json",
                "--min-samples",
                "2",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn parses_progressive_preview_options_and_rejects_conflicts() {
        let defaults =
            Cli::try_parse_from(["toaster", "stream-preview", "scene.json", "--progressive"])
                .unwrap();
        let Command::StreamPreview {
            progressive,
            batch_samples,
            target_samples,
            ..
        } = defaults.command
        else {
            panic!("expected stream-preview command");
        };
        assert!(progressive);
        assert_eq!(batch_samples, None);
        assert_eq!(target_samples, None);

        let explicit = Cli::try_parse_from([
            "toaster",
            "stream-preview",
            "scene.json",
            "--progressive",
            "--batch-samples",
            "2",
            "--target-samples",
            "256",
        ]);
        assert!(explicit.is_ok());

        for args in [
            vec![
                "toaster",
                "stream-preview",
                "scene.json",
                "--batch-samples",
                "2",
            ],
            vec![
                "toaster",
                "stream-preview",
                "scene.json",
                "--progressive",
                "--batch-samples",
                "0",
            ],
            vec![
                "toaster",
                "stream-preview",
                "scene.json",
                "--progressive",
                "--samples",
                "2",
            ],
            vec![
                "toaster",
                "stream-preview",
                "scene.json",
                "--progressive",
                "--loop-duration",
                "2",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn gpu_render_does_not_accept_preview_options() {
        assert!(Cli::try_parse_from([
            "toaster",
            "gpu-render",
            "scene.json",
            "--out",
            "image.png",
            "--stream",
        ])
        .is_err());
    }
}
