mod cli;

use clap::Parser;
use cli::{Cli, Command};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::CpuRender { scene_path, out } => {
            let scene = toaster_scene::load_scene(&scene_path)?;
            println!("Scene: {}", scene_path.display());
            println!("Output: {}", out.display());
            println!("Resolution: {}x{}", scene.render.width, scene.render.height);
            println!("Samples: {}", scene.render.samples);

            let start = Instant::now();
            let image = toaster_cpu::render(&scene);
            image.save_png(&out)?;
            println!("Rendered in {:.2?}", start.elapsed());
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
