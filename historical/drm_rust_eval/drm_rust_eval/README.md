# DRM Rust Convergence Prototype

A std-only Rust prototype of the updated DRM task-runtime mechanics discussed in the design review.

## What is implemented

- **Fixed canonical vocabulary**: 14 root operations. Derived vocabulary cannot introduce a new leaf.
- **Recursive derived vocabulary**: every learned word expands finitely to canonical roots.
- **Global MDL growth gate**: a derived word is admitted only when it reduces whole-corpus description length after paying its own definition cost.
- **Bounded hot context**: active task concepts have an LRU cap; persistent historical task IR is separate.
- **Ancestral execution**: an old state is hydrated only when an episode explicitly requires historical semantic context.
- **Forward integration**: after ancestral recovery, the task is installed in current context, so the immediate repeat does not recover again.
- **Local repair**: changed workflows repair only their differing middle region.
- **Instrumentation**: semantic decisions, recoveries, local repairs, structural changes, runtime storage, derived vocabulary count, and vocabulary integrity.
- **Baselines**: stateless planner, task/checkpoint replay, flat structural-template cache.

## Build and run on Linux

```bash
cargo test
cargo run --release
```

No external crates are required.

## Important validation note

The execution sandbox used to create this prototype did not contain `rustc`/`cargo` and had no shell network access to install them. Therefore the Rust source was not compiled in that sandbox. `validate_mirror.py` is an executable behavioral mirror of the same algorithm and was used for the reported tests. Do not treat the Rust source as compile-verified until `cargo test` has been run on a Rust-equipped host.
