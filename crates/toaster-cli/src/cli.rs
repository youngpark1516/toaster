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
    Server {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 7878)]
        port: u16,
    },
    Info,
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
}
