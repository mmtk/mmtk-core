# Pull Requests

## API Breaking Changes

If a PR includes API breaking changes, it is the responsibility of the MMTk team to make sure
the officially supported bindings are updated to accomodate the changes.
A team member may open a PR in the bindings him/herself if the required
changes are trivial enough. Otherwise he/she may ask the team member that is responsible for a specific
binding to request an update. The corresponding binding PRs need to be ready to merge before the mmtk-core PR
can be merged.

### Testing PRs with API breaking changes

If an MMTk core PR should be tested with other bindings PRs, one can specify the binding branch that
should be tested with by adding a comment like below (see https://github.com/mmtk/mmtk-core/blob/master/.github/workflows/pr-binding-refs.yml).
If there are multiple comments that match, the first one is effective. If the info is missing for
a binding, the default repo (`mmtk/mmtk-X`) and branch (`master`) will be used instead.
```
binding-refs
OPENJDK_BINDING_REPO=xx/xx
OPENJDK_BINDING_REF=xxxxxx
JIKESRVM_BINDING_REPO=xx/xx
JIKESRVM_BINDING_REF=xxxxxx
V8_BINDING_REPO=xx/xx
V8_BINDING_REF=xxxxxx
JULIA_BINDING_REPO=xx/xx
JULIA_BINDING_REF=xxxxxx
RUBY_BINDING_REPO=xx/xx
RUBY_BINDING_REF=xxxxxx
```

### Merging a PR with API breaking changes

If an MMTk core PR includes API breaking changes, the corresponding binding PRs depends on an mmtk-core commit in the PR branch. As we
use squashing merging, the commit in the PR branch will disappear once the mmtk-core PR is merged. When the mmtk-core PR is merged,
we will have a new commit in `mmtk-core` master. We will need to fix the mmtk dependency in the binding PRs to point to the new commit,
and then merge the binding PRs.

#### Auto merging process

This process should be done automatically by [`auto-merge.yml`](https://github.com/mmtk/mmtk-core/blob/master/.github/workflows/auto-merge.yml)
when an mmtk-core PR is merged and the `binding-refs` comment is present.

1. Make sure there is no other PR in this merging process. If so, resolve those first.
2. Make sure all the PRs (the mmtk-core PR, the binding PRs, and the associated PRs in the VM repo if any) are ready to merge.
3. Make sure there is a comment that provides `binding-refs` for all the binding PRs.
4. For each binding PR that we need to merge:
   1. If the binding PR has an assocate PR in the VM repo, merge the VM PR first. Once it is merged, we will have a commit hash (we refer to it as `{vm_commit}`).
   2. Update `mmtk/Cargo.toml` in the binding:
      * Find the section `[package.metadata.{binding-name}]`.
      * Update the field `{binding-name}_repo` if necessary. It should point to our VM fork, such as `https://github.com/mmtk/{binding-name}.git`.
      * Update the field `{binding-name}_version`. It should point to the new commit hash `{vm_commit}`.
      * Commit the change.
5. Merge the mmtk-core PR.
6. When a new commit is pushed to `master`, `auto-merge.yml` will be triggered.
7. The binding PRs should be updated and auto merge will be eanbled for the PR. Keep an eye until the PRs are all merged. Resolve any
   issue that prevents the PR from being auto merged (e.g. flaky tests).

#### Manual merging process

If `auto-merge.yml` failed for any reason, or if we have to manually merge binding PRs, this is the process to follow:

1. Follow Step 1-5 in the auto merging process. (Step 3 is optional)
2. When a new commit is pushed to `master`, we record the commit hash (as `{mmtk_core_commit}`).
3. For each binding PR that we need to merge:
   1. Update `mmtk/Cargo.toml` in the binding:
      * Find the `mmtk` dependency under `[dependencies]`.
      * Update the field `git` if necessary. It should point to our mmtk-core repo, `https://github.com/mmtk/mmtk-core.git`.
      * Update the field `rev`. It should point to the new mmtk-core commit hash `{mmtk_core_commit}`.
      * Update `mmtk/Cargo.lock` by building the Rust project again. If the binding needs to choose a GC plan by feature, use
        any supported plan. So this step is slightly different for different bindings:
        * OpenJDK, Ruby: `cargo build`
        * JikesRVM: `cargo build --features nogc --target i686-unknown-linux-gnu`
        * V8: `cargo build --features nogc`
        * Julia: `cargo build --features immix`
      * Check in both `mmtk/Cargo.toml` and `mmtk/Cargo.lock`, and commit.
    2. Merge the PR once it can be merged.
