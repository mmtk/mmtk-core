# Enabling Link Time Optimization (LTO) with MMTk

MMTk's API is designed with an assumption that LTO will be enabled for a performant build.
It is essential to allow the Rust compiler to optimize across the crate boundary between the binding crate and mmtk-core.
LTO allows inlining for both directions (from mmtk-core to the binding, and from the binding to mmtk-core),
and allows further optimization such as specializing and constant folding for the `VMBinding` trait.

We suggest enabling LTO for the release build in the binding's manifest (`Cargo.toml`) by adding a profile for the release build,
so LTO is always enabled for a release build.

```toml
[profile.release]
lto = true
```

If your binding project is a Rust binary (e.g. the VM is written in Rust), this should be enough. However, if your binding project
is a library, there are some limitations with cargo that you should be aware of.


## Binding as a library

Cargo only allows LTO for certain crate types. You will need to specify the crate type properly, otherwise cargo may skip LTO without
any warning or error.

```toml
[lib]
...
# be careful - LTO is only allowed for certain crate types
crate-type = ["cdylib"]
```

At the time of writing, cargo has some limitations about LTO with different crate types:
1. LTO is only allowed with `cdylib` and `staticlib` (other than `bin`).
Check the code of [`can_lto`](https://github.com/rust-lang/cargo/blob/5f40a97e5c85affecfbc4fde67fc06bf188c07db/src/cargo/core/compiler/crate_type.rs#L33)
for your Rust version to clarify.
2. If the `crate-type` field includes any type that LTO is not allowed, LTO will be skipped for all the libraries generated (https://github.com/rust-lang/rust/issues/51009).
For example, if you have `crate-type = ["cdylib", "rlib"]` and cargo cannot do LTO for `rlib`, LTO will be skipped for `cdylib` as well.
So only keep the crate type that you actually need in the `crate-type` field.
