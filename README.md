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
As the Rust language and its libraries (crates) are frequently evolving, we recommend using the nightly toolchain specified in the [`rust-toolchain`](rust-toolchain) file.

```console
$ # replace nightly-YYYY-MM-DD with the toolchain version specified in mmtk-dev-env
$ export RUSTUP_TOOLCHAIN=nightly-YYYY-MM-DD

$ cargo build
```

MMTk also provides a list of optional features that users can choose from. 
A full list of available features can be seen by examining [`Cargo.toml`](Cargo.toml).
By passing the `--features` flag to the Rust compiler,
we conditionally compile feature-related code.
For example, you can optionally enable sanity checks by adding `sanity` to the set of features
you want to use.

You can pass the `--release` flag to the `cargo build` command to use the
optimizing compiler of Rust for better performance.

The artefact produced produced by the build process can be found under
`target/debug` (or `target/release` for the release build).

[`ci-build.sh`](.github/scripts/ci-build.sh) shows the builds tested by the CI.

## Usage

MMTk does not run standalone. You would need to integrate MMTk with a language implementation.
You can either try out one of the VM bindings we have been working on, or implement your own binding in your VM for MMTk.
You can also implement your own GC algorithm in MMTk, and run it with supported VMs.
You can find an up-to-date API document for mmtk-core here: https://www.mmtk.io/mmtk-core/mmtk.

### Try out our current bindings

We maintain three VM bindings for MMTk. These bindings are accessible in the following repositories:

* [OpenJDK](https://github.com/mmtk/mmtk-openjdk),
* [JikesRVM](https://github.com/mmtk/mmtk-jikesrvm),
* [V8](https://github.com/mmtk/mmtk-v8).

For more information on these bindings, please visit their repositories.

### Implement your binding

MMTk provides a bi-directional interface with the language VM.

1. MMTk exposes a set of [APIs](src/memory_manager.rs). The language VM can call into MMTk by using those APIs.
2. MMTk provides a trait [`VMBinding`](src/vm/mod.rs) that each language VM must implement. MMTk use `VMBinding` to call into the VM.

To integrate MMTk with your language implementation, you need to provide an implementation of `VMBinding`, and
you can optionally call MMTk's API for your needs.

For more information, you can refer to our [porting guide](https://www.mmtk.io/mmtk-core/portingguide) for VM implementors.

### Implement your GC

MMTk is a suite of various GC algorithms (known as plans in MMTk). MMTk provides reusable components that make it easy
to construct your own GC based on those components. For more information, you can refer to our [tutorial](https://www.mmtk.io/mmtk-core/tutorial)
for GC implementors.

## Tests

We use both unit tests and VM binding tests to test MMTk in the CI.

### Unit tests

MMTk uses Rust's testing framework for unit tests. For example, you can use the following to run unit tests.

```console
$ cargo test
```

A full list of all the unit tests we run in our CI can be found [here](.github/scripts/ci-test.sh).

### VM binding tests

MMTk is also tested with the VM bindings we are maintaining by running standard test/benchmark suites for the VMs.
For details, please refer to each VM binding repository.

## Contributing to MMTk

Thank you for your interest in contributing to MMTk. We appreciate all the contributors. There are multiple ways you can help and contribute to MMTk.

### Reporting a bug

If you encounter any bug when using MMTk, you are welcome to submit an issue ([mmtk-core issues](https://github.com/mmtk/mmtk-core/issues)) to report it. We would suggest including essential information to reproduce and investigate the bug, such as the revisions of mmtk-core and the related bindings, the command line arguments used to build, and the command line executed to reproduce the bug.

### Submit a pull request

If you would like to upstream non-trivial changes to MMTk, we suggest first getting involved in the discussion of the related [Github issues](https://github.com/mmtk/mmtk-core/issues), or talking to any MMTk team member on [our Zulip](https://mmtk.zulipchat.com/). This makes sure that others know what you are up to, and makes it easier for your changes to get accepted to MMTk.

Generally we expect a pull request to meeting the following requirements before it can be merged:
1. The PR includes only one change. You can break down large pull requests into separate smaller ones.
2. The code is well documented and a PR only introduces unsafe code where it is a must.
3. The PR passes the mmtk-core unit tests and complies with the coding style. We have scripts in `.github/scripts` that are used by our Github action to run those checks for each PR.
4. The PR passes all the binding tests. We run benchmarks with bindings to test mmtk-core. A new pull request should not break bindings, as we ensure that our supported bindings always work with the latest mmtk-core. If a pull request makes changes that require the bindings to be updated correspondingly, you can approach the MMTk team on [our Zulip](https://mmtk.zulipchat.com/) and seek help from them to update the bindings.
