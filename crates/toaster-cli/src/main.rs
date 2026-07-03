mod cli;

use clap::Parser;
use cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::CpuRender { scene_path, out } => {
            println!(
                "CPU render placeholder: {} -> {}",
                scene_path.display(),
                out.display()
            );
        }
        Command::GpuRender { scene_path, out } => {
            println!(
                "GPU render placeholder: {} -> {}",
                scene_path.display(),
                out.display()
            );
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
