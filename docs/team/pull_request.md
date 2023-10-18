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

This process will be done automatically by [`auto-merge.yml`](https://github.com/mmtk/mmtk-core/blob/master/.github/workflows/auto-merge.yml)
when the `binding-refs` comment is present.
