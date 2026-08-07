# Toaster documentation

This directory contains focused guides for Toaster users and contributors.
Planned physics and later rendering work is always labeled as future work.

## Start here

- [Root README](../README.md): ownership, quickstart, architecture, and measured performance.
- [Scene format](scene_format.md): complete JSON input schema, defaults, validation, assets, and examples.
- [Shader reference](shader_reference.md): active WGSL layouts, bindings, and algorithms.

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

## Documentation maintenance

When behavior or a public interface changes, update the relevant source Rustdoc
and focused guide in the same change. Keep operational procedures in their
existing documents and link to them instead of duplicating source-level detail.
