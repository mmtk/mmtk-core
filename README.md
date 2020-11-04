# MMTk

[![crates.io](https://img.shields.io/crates/v/mmtk.svg)](https://crates.io/crates/mmtk)
[![docs.rs](https://docs.rs/mmtk/badge.svg)](https://docs.rs/mmtk/)
[![project chat](https://img.shields.io/badge/zulip-join_chat-brightgreen.svg)](https://mmtk.zulipchat.com/)

MMTk is a framework for the design and implementation of memory managers.
This repository hosts the Rust port of MMTk.

## Contents

* [Requirements](#requirements)
* [Build](#build)
* [Usage](#Usage)
* [Tests](#tests)

## Requirements

We maintain an up to date list of the prerequisite for building MMTk and its bindings in the [mmtk-dev-env](https://github.com/mmtk/mmtk-dev-env) repository.

## Build

Building MMTk requires a nightly Rust toolchain.
As the Rust language and its libraries (crates) are frequently evolving, we recommend using the nightly toolchain specified in the [mmtk-dev-env](https://github.com/mmtk/mmtk-dev-env).

```console
$ # replace nightly-YYYY-MM-DD with the toolchain version specified in mmtk-dev-env
$ Export RUSTUP_TOOLCHAIN=nightly-YYYY-MM-DD

$ cargo build --features <space separated features>
```

You must specify a GC plan as a feature at build time.
Currently, there are two different plans to choose from:

* `--features nogc` for NoGC, and
* `--features semispace` for SemiSpace.

A full list of available features can be seen by examining [`Cargo.toml`](Cargo.toml).
By passing the `--features` flag to the Rust compiler,
we conditionally compile plan-specific code.
You can optionally enable sanity checks by adding `sanity` to the set of features
you want to use.

You can pass the `--release` flag to the `cargo build` command to use the
optimizing compiler of Rust for better performance.

The artefact produced produced by the build process can be found under
`target/debug` (or `target/release` for the release build).

[`ci-build.sh`](.github/scripts/ci-build.sh) shows the builds tested by the CI.

## Usage

MMTk does not run standalone. You would need to integrate MMTk with a language implementation. You can either try out one of the VM bindings we have been working on, or implement your own binding in your VM for MMTk.

### Try out our current bindings

We maintain three VM bindings for MMTk. These bindings are accessible in the following repositories:

* [OpenJDK](https://github.com/mmtk/mmtk-openjdk),
* [JikesRVM](https://github.com/mmtk/mmtk-jikesrvm),
* [V8](https://github.com/mmtk/mmtk-v8).

For more information on these bindings, please visit their repositories.

### Implement your binding

MMTk provides a bi-directional interface with the language VM.

1. MMTk exposes a set of [APIs](src/mm/memory_manager.rs). The language VM can call into MMTk by using those APIs.
2. MMTk provides a trait [`VMBinding`](src/vm/mod.rs) that each language VM must implement. MMTk use `VMBinding` to call into the VM.

To integrate MMTk with your language implementation, you need to provide an implementation of `VMBinding`, and
you can optionally call MMTk's API for your needs.

## Tests

We use both unit tests and VM binding tests to test MMTk in the CI.

### Unit tests

MMTk uses Rust's testing framework for unit tests. For example, you can use the following to run unit tests for the `nogc` plan.

```console
$ cargo test --features nogc
```

A full list of all the unit tests we run in our CI can be found [here](.github/scripts/ci-test.sh).

### VM binding tests

MMTk is also tested with the VM bindings we are maintaining by running standard test/benchmark suites for the VMs.
For details, please refer to each VM binding repository.
