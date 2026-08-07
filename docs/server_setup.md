# Cluster setup

This is the operational cluster/SSH guide. See the [documentation index](README.md) for HTTP interfaces and renderer internals.

Before rendering on a compute node, check the available GPU and toolchain:

```sh
nvidia-smi
rustc --version
cargo --version
vulkaninfo --summary
```

The Vulkan check is optional because some login nodes omit the utility. Run `scripts/server_check.sh` for a non-destructive summary. Keep hostnames, credentials, SSH paths, and local scheduler configuration outside the repository.

## Live preview smoke test

On the GPU machine, start a finite preview:

```sh
cargo run -p toaster-cli -- stream-preview scenes/006_rotating_cube.json \
  --host 127.0.0.1 --port 7878 --fps 12 --duration 10
```

While it is running, verify the server from another shell:

```sh
curl --fail http://127.0.0.1:7878/healthz
```

Open <http://127.0.0.1:7878/> to view the browser preview. The page reads its MJPEG data from `/stream`.

The live metrics used by the page are also available directly:

```sh
curl --fail http://127.0.0.1:7878/status
```

For an indefinite preview, omit `--duration`; stop it with Ctrl+C:

```sh
cargo run -p toaster-cli -- stream-preview scenes/006_rotating_cube.json --fps 12
```

For a lower-cost looping mesh preview:

```sh
cargo run -p toaster-cli -- stream-preview scenes/004_mesh.json \
  --fps 12 --width 800 --height 800 --samples 16 --max-bounces 6 \
  --loop-duration 2
```

Add bounded adaptive sampling when maintaining the requested FPS matters more than a fixed sample count:

```sh
cargo run -p toaster-cli -- stream-preview scenes/004_mesh.json \
  --fps 12 --width 800 --height 800 --max-bounces 6 \
  --samples 16 --adaptive-samples --min-samples 2 --max-samples 32 \
  --loop-duration 2
```

For a static scene, accumulate progressive batches up to an exact target:

```sh
cargo run -p toaster-cli -- stream-preview scenes/010_environment_map.json \
  --progressive --batch-samples 2 --target-samples 256 \
  --adaptive-samples --min-samples 1 --max-samples 8 --fps 12
```

The last partial batch is clamped to the target. After convergence, the server
continues serving the final frame until Ctrl+C or `--duration` expires.

No preview frames are saved to disk. Existing `gpu-render --out ...` commands continue to provide PNG output.

To view a loopback-bound preview from a local browser while rendering remotely:

```sh
ssh -L 7878:127.0.0.1:7878 <remote>
```

Keep the tunnel open, run `stream-preview` on the remote host with `--host 127.0.0.1 --port 7878`, and browse to <http://127.0.0.1:7878/> locally.
