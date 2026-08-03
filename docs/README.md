# Toaster documentation

This directory is the navigation hub for Toaster users, contributors, and maintainers. The documents describe the current working implementation. Planned BVH and physics work is always labeled as future work.

## Start here

- [Project guide](project_guide.md): build, run, preview, and use the current renderer.
- [Codebase reference](codebase_reference.md): architecture, ownership, runtime flows, modules, and the complete production-function catalog.
- [Public API](public_api.md): exported Rust interfaces plus CLI commands and HTTP routes.
- [Scene format](scene_format.md): complete JSON input schema, defaults, validation, assets, and examples.
- [Shader reference](shader_reference.md): active WGSL layouts, bindings, algorithms, and function catalog.

## Operations and design

- [Architecture](architecture.md): short architectural overview.
- [Rendering notes](rendering_notes.md): path-tracing implementation notes.
- [GPU benchmarking and logging](benchmarking.md): repeatable performance reports and structured logs.
- [Cluster and streaming setup](server_setup.md): Slurm GPU-node and SSH-forwarding workflow.
- [Roadmap](roadmap.md): completed and future milestones.

## Generated implementation reference

Rustdoc is generated locally under ignored `target/doc`:

```sh
./scripts/check_docs.sh
```

The check builds documentation for private items with warnings denied, validates intra-doc links, and runs workspace doctests. Open `target/doc/toaster_gpu/index.html` (or another crate index) after it completes. Generated HTML must not be committed.

## Documentation ownership

When behavior or a public interface changes, update the relevant source Rustdoc and the focused guide in the same change. Keep operational procedures in their existing focused documents and link them here instead of duplicating them. The comprehensive reference records implementation detail; the public API and scene/shader references are the quick-lookup surfaces.
