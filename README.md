# MMTk

MMTk is a framework for the design and implementation of memory managers.
This repository hosts the Rust port of MMTk.

## Contents

* [Requirements](#requirements)
* [Build](#build)
* [Usage](#Usage)
* [Tests](#tests)
* [Bindings](#Bindings)

## Requirements

We maintain an up to date list of the prerequisite for building MMTk and its bindings in the [mmtk-docker](https://github.com/mmtk/mmtk-docker) repository.

## Build

Buidling MMTk needs a nightly Rust toolchain.
As the Rust language and its libraries (crates) are frequently evolving, we recommand using the nightly toolchain specified in the [mmtk-docker](https://github.com/mmtk/mmtk-docker).

```bash
# replace nightly-YYYY-MM-DD with the correct toolchain version
Export RUSTUP_TOOLCHAIN=nightly-YYYY-MM-DD

cargo build --features <space seperated features>
```

The available features can be seen by examining `Cargo.toml`.
By passing the `--features` flag to the Rust compiler,
we conditionally compile VM or plan specific code.
You can optionally enable sanity checks by add `sanity` to the set of features
you want to use.

Currently, there are two different plans to choose from:

* `--features nogc` for NoGC, and 
* `--features semispace` for SemiSpace.

You can pass the `--release` flag to the `cargo build` command to use the
optimizing compiler of Rust for better performance.

The artefact produced produced by the build process can be found under
`target/debug` (or `target/release` for the release build).

## Usage

The API exposed by MMTk can be found under `api/mmtk.h`.
A client of the memory manager can use MMTk like a C library in the standard way.
A simple example client that uses MMTk for just allocation can be found under
`examples/main.c`.

## Tests

The unit tests of MMTk are written in Rust, in the same location as the unit they are testing.
Each test is marked by `#[test]` as required by Rust.

The following commands may be used to run unit tests for a specific set of features:

```bash
# replace nightly-YYYY-MM-DD with the correct toolchain version
Export RUSTUP_TOOLCHAIN=nightly-YYYY-MM-DD

cargo test --features <space seperated features>
```

For instance, the unit tests for the NoGC plan may be run as:

```bash
cargo build --features nogc
```

Currently, the CI runs all the MMTk unit tests.

## Bindings

We are maintaining three VM bindings available for MMTk. These bindings are accessible in the following repositories:

* [OpenJDK](https://github.com/mmtk/mmtk-openjdk),
* [JikesRVM](https://github.com/mmtk/mmtk-jikesrvm),
* [V8](https://github.com/mmtk/mmtk-v8).

For more information on these bindings, please visit their repositories.
