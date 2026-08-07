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

The browser UI is at <http://127.0.0.1:7878/>. `/stream` serves MJPEG,
`/healthz` is a health check, and `/status` returns live metrics. Omit
`--duration` to run until Ctrl+C.

For static scenes, `--progressive --batch-samples 2 --target-samples 256`
accumulates to an exact target. For bounded adaptive work, add
`--adaptive-samples --min-samples 1 --max-samples 8`. Progressive mode is not
valid for animation. Preview frames are kept in memory; use `gpu-render --out`
for PNG output.

## View a remote node locally

Keep this tunnel open from the local machine:

```sh
ssh -L 7878:127.0.0.1:7878 <remote>
```

Run the preview on the remote host with `--host 127.0.0.1 --port 7878`, then
open <http://127.0.0.1:7878/> locally.
