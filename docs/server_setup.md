# Cluster and remote preview

On the allocated GPU node, verify the adapter and Rust toolchain:

```sh
nvidia-smi
rustc --version
cargo --version
./scripts/server_check.sh
```

Keep hostnames, credentials, SSH paths, and scheduler-specific configuration
outside the repository.

## Start a preview

```sh
cargo run -p toaster-cli -- stream-preview scenes/006_rotating_cube.json \
  --host 127.0.0.1 --port 7878 --fps 12 --duration 10
```

The browser UI is at <http://127.0.0.1:7878/>. `/stream` serves MJPEG and
`/healthz` is a health check. Omit `--duration` to run until Ctrl+C.

For static scenes, `--progressive --batch-samples 2 --target-samples 256`
accumulates to an exact target. For bounded adaptive work, add
`--adaptive-samples --min-samples 1 --max-samples 8`. Progressive mode is not
valid for animation or enabled physics. Normal non-progressive MJPEG preview
supports both. Preview frames are kept in memory; use `gpu-render --out` for PNG
output.

The rigid-body demo can be previewed and looped with:

```sh
cargo run --release -p toaster-cli -- stream-preview \
  scenes/011_physics_rigid_bodies.json \
  --host 127.0.0.1 --port 7878 --fps 12 --loop-duration 5
```

Each loop resets and deterministically replays the physics world. Physical poses
repeat while path-tracing noise may differ because render frame indices remain
monotonic.

## View a remote node locally

Keep this tunnel open from the local machine:

```sh
ssh -L 7878:127.0.0.1:7878 <remote>
```

Run the preview on the remote host with `--host 127.0.0.1 --port 7878`, then
open <http://127.0.0.1:7878/> locally.

Use `--host 0.0.0.0` only when a separate cluster or login node must connect to
the compute node through an internal hostname. In that case, run Toaster on the
compute node with the wildcard binding and tunnel through the login node:

```sh
ssh -N -L 7878:<compute-node-internal-hostname>:7878 <login-node>
```
