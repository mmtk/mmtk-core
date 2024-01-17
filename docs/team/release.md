# Release

This documents how MMTk is currently released in the pre-1.0 stage.

## Release Cycle

We maintain a 6-week release cycle. A release is usually cut at the end of a week (Friday).

## Release Scope

MMTk releases include MMTk core and the officially supported bindings. They share the same version number as MMTk core.

The current releases include the following bindings:
* OpenJDK
* JikesRVM

The current releases do not include the following bindings:
* Julia: We have made good progress on the binding development, and we will include it soon.
* Ruby: We have made good progress on the binding development, and we will include it soon.
* V8: We currently lack of resources to work on the binding.

## Release Process

### Create PRs to bump the version number

Create pull requests for each repository (mmtk-core, and binding repos that will be included in the release).
We use semantic versioning for MMTk core, and use the same version number for all the bindings in the same release.
If the current version is `0.X.x`, the new version should be `0.X+1.0`.

#### MMTk core

The PR should include these changes:

1. Bump version in `Cargo.toml`.
2. Bump version in `macros/Cargo.toml`. Use the new version for the `mmtk-macros` dependency in `Cargo.toml`.
3. Update `CHANGELOG.md`:
   1. Add a section for the new version number and the cut-off date (when the PR is created)
   2. Add change logs for the release. The following shows one convenient way to do it. If there is a better way, we should adopt.
      1. Auto generate the list of changes for the release on Github. Click on [`releases`](https://github.com/mmtk/mmtk-core/releases),
         then click [`Draft a new release`](https://github.com/mmtk/mmtk-core/releases/new). Enter the new version tag,
         and the `Generate release notes` button should be avaialble. Copy the notes as the change logs to `CHANGELOG.md`.
         Close the release page without tagging a release.
      2. Categorize the changes in `CHANGELOG.md`. We use these categories: Plan, Policy, Allocator, Scheduler, API, Documentation, CI, Misc.
4. Update the pinned Rust version in `rust-toolchain` if necessary.
   1. Talk with system admin for our CI machines, and check if there is a newer Rust version that we should be using.
   2. If we update to a new Rust version, make necessary changes to the code base.

#### Bindings

The PR should include these changes:

1. Bump version in `mmtk/Cargo.toml`. Use the same version as MMTk core.
2. Update `CHANGELOG.md`, similar to the process in MMTk core.
3. Update the pinned Rust version in `rust-toolchain` if the Rust version is updated for MMTk core.
4. Update the dependency of `mmtk` in `mmtk/Cargo.toml` to use the MMTk core PR. Update `Cargo.lock`.

#### Merging PRs

We should have a mmtk-core PR and multiple binding PRs. When all the PRs are approved, we can start merging the PRs.
The merging is the same as ['Merging a PR with API breaking changes' in pull_request.md](./pull_request.md#merging-a-pr-with-api-breaking-changes).

### Tag releases

Once the PRs are merged, we can tag releases on Github.

1. Go to 'Create a new release' for each involved repository. E.g. https://github.com/mmtk/mmtk-core/releases/new for `mmtk-core.`
2. Enter the new version (prefixed with `v`) in the box of 'Choose a tag'. Use the default 'target' (`master`).
3. Enter the release title
   * `MMTk 0.x.0`
   * `MMTk OpenJDK Binding 0.x.0`
4. Copy the markdown section for this version in the `CHANGELOG.md` as the description for the release.
5. Tick 'Set as a pre-release'.
6. Click 'Publish release'.

### Post release checklist

1. Keep an eye on the badges in [README](https://github.com/mmtk/mmtk-core#mmtk)
   * crates.io: Once a release is tagged for `mmtk-core`, [cargo-publish.yml](https://github.com/mmtk/mmtk-core/blob/master/.github/workflows/cargo-publish.yml) should be trigger to publish `mmtk-core` to `crates.io`. https://crates.io/crates/mmtk should show the new version.
   * docs: Document hosting: Once `mmtk-core` is published to `crates.io`, a job should be queue'd for document generation on `docs.rs`. https://docs.rs/mmtk/latest/mmtk/ should show
   the docs for the new version once the generation is done.
2. Keep an eye on the CI status for the latest commit in MMTk core.
3. Do point release for fixing severe issues. Currently we normally do not need point releases. Normal bug fixes or any other issues can be fixed in the next release.
   But in rare cases, such as the current tagged release cannot build, cannot run, or it somehow fails in publishing, we may need to do a point release.

### Point release

   1. Create a pull request to fix the issue.
   2. Create a pull request to bump the version number, following the same process [above](#create-prs-to-bump-the-version-number).
   3. Once the PRs are merged,
      * Create a branch in the main repository based on the last release `v0.x.y` (from `master` or from the last point release branch), named with the new point release version, such as `v0.x.y+1`.
      * Cherry pick commits from `master` that should be included in the new point release.
      * Tag a release from the branch, following the same process [above](#tag-releases).
      * If there is no other commit in `master` yet, there is no need to create a different branch for the release, and we can tag a release from `master`.
