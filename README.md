# MMTk

MMTk is a framework for the design and implementation of memory managers.
This repo hosts the Rust port of MMTk.

## Build
```bash
cargo +nightly build --no-default-features --features <space seperated features>
```

The available features can be seen by examining `Cargo.toml`.
Currently, there are two different VMs to choose from (JikesRVM and OpenJDK),
and there are three different plans to choose from (NoGC, SemiSpace and G1).
By passing the `--features` flag to the Rust compiler,
we conditionally compile VM or plan specific code.
You can optionally enable sanity checks by add `sanity` to the set of features
you want to use.

You can pass the `--release` flag to the `cargo build` command to use the
optimizing compiler of Rust for better performance.

Cross compilation can be done by using the `--target` flag.

The artefact produced produced by the build process can be found under
`target/debug` (or `target/release` for release build).

## Usage
The API exposed by MMTk can be found under `api/mmtk.h`.
A client of the memory manager can use MMTk like a C library in the standard way.
A simple example client that uses MMTk for just allocation can be found under
`examples/main.c`.

## Tests
The Rust unit tests can be found under `tests`.
Currently,
the CI will run all the Rust unit tests.
The CI will also build both the 32-bit and 64-bit versions of MMTk for each plan
to test the `alloc` function.

## VM specific notes
### JikesRVM
Please DO NOT build MMTk manually,
as machine generated code is involved during the build process (e.g. `src/vm/jikesrvm/entrypoint.rs` and `src/vm/jikesrvm/inc.asm`).
Instead, please invoke the `buildit` script from JikesRVM.

An outside collaborator should be able to run our CI if the PR does not change workflow. 
