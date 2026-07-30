# Cluster setup

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

For an indefinite preview, omit `--duration`; stop it with Ctrl+C:

```sh
cargo run -p toaster-cli -- stream-preview scenes/006_rotating_cube.json --fps 12
```

No preview frames are saved to disk. Existing `gpu-render --out ...` commands continue to provide PNG output.

To view a loopback-bound preview from a local browser while rendering remotely:

```sh
ssh -L 7878:127.0.0.1:7878 <remote>
```

Keep the tunnel open, run `stream-preview` on the remote host with `--host 127.0.0.1 --port 7878`, and browse to <http://127.0.0.1:7878/> locally.
