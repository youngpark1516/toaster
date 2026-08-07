# Documentation

The root [README](../README.md) is the project overview and contains the measured
BVH results. These guides cover details that are useful while operating or
changing the renderer:

- [Architecture](architecture.md): data flow and the main engineering decisions.
- [Scene format](scene_format.md): JSON schema, defaults, validation, and assets.
- [Shader reference](shader_reference.md): Rust/WGSL layouts and shader invariants.
- [Benchmarking](benchmarking.md): reproducible measurements and report format.
- [Cluster setup](server_setup.md): GPU-node checks and remote preview setup.

Keep implementation details in source comments and Rustdoc. Run
`./scripts/check_docs.sh` to validate documentation and doctests.
