# Contributing

Create a focused branch and keep changes reviewable. Add tests when behavior is introduced. Do not commit credentials, hostnames, SSH paths, cluster configuration, generated Rustdoc, or large generated renders.

## Required validation

Run the complete local checks before requesting review:

```sh
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
./scripts/check_docs.sh
```

Changes requiring a GPU, FFmpeg, network listener, or cluster should also receive a proportional manual smoke test. Document the command and environment in the pull request; do not commit machine-specific output from `out/`.

## Documentation workflow

The [documentation index](docs/README.md) explains where information belongs. Any production interface or behavior change must update:

- Rustdoc on the affected public and private items;
- comments on affected WGSL structures, bindings, functions, and entry points;
- the focused user/reference document that describes the behavior; and
- the function catalog in [docs/codebase_reference.md](docs/codebase_reference.md) when functions are added, renamed, removed, or materially repurposed.

`./scripts/check_docs.sh` generates private-item HTML under ignored `target/doc`, denies Rustdoc warnings and broken intra-doc links, and runs workspace doctests. Do not commit `target/doc`.

Tests should be summarized at module or guide level; individual test functions do not require documentation comments.
