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

        #[arg(long)]
        out: PathBuf,

        #[arg(long)]
        fps: Option<u32>,

        #[arg(long)]
        duration: Option<f32>,
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
}
