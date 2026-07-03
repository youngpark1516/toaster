# Cluster setup

Before rendering on a compute node, check the available GPU and toolchain:

```sh
nvidia-smi
rustc --version
cargo --version
vulkaninfo --summary
```

The Vulkan check is optional because some login nodes omit the utility. Run `scripts/server_check.sh` for a non-destructive summary. Keep hostnames, credentials, SSH paths, and local scheduler configuration outside the repository.

