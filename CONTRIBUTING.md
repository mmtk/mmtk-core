# Contributing to MMTk

Thank you for your interest in contributing to MMTk. We appreciate all the contributors. There are multiple ways you can help and contribute to MMTk.

## Reporting a bug

If you encounter any bug when using MMTk, you are welcome to submit an issue ([mmtk-core issues](https://github.com/mmtk/mmtk-core/issues)) to report it. We would suggest including essential information to reproduce and investigate the bug, such as the revisions of mmtk-core and the related bindings, the command line arguments used to build, and the command line executed to reproduce the bug.

## Submit a pull request

If you would like to upstream non-trivial changes to MMTk, we suggest first getting involved in the discussion of the related [Github issues](https://github.com/mmtk/mmtk-core/issues), or talking to any MMTk team member on [our Zulip](https://mmtk.zulipchat.com/). This makes sure that others know what you are up to, and makes it easier for your changes to get accepted to MMTk.

Generally we expect a pull request to meeting the following requirements before it can be merged:
1. The PR includes only one change. You can break down large pull requests into separate smaller ones.
2. The code is well documented.
3. The PR does not introduce unsafe Rust code unless necessary. Whenever introducing unsafe code, the contributor must elaborate why it is necessary.
4. The PR passes the mmtk-core unit tests and complies with the coding style. We have scripts in `.github/scripts` that are used by our Github action to run those checks for each PR.
5. The PR passes all the binding tests. We run benchmarks with bindings to test mmtk-core. A new pull request should not break bindings, as we ensure that our supported bindings always work with the latest mmtk-core. If a pull request makes changes that require the bindings to be updated correspondingly, you can approach the MMTk team on [our Zulip](https://mmtk.zulipchat.com/) and seek help from them to update the bindings.