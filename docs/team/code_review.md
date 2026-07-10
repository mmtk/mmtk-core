# Code Review

Code review is very important for the project. Any change/PR needs to be reviewed and approved before
merging into the mainline.

## Nominate a Reviewer

The contributor may nominate a reviewer. If a pull request has no nominated reviewer, a team member
may nominate a reviewer instead (including self-assigning). Anyone with review access may be a reviewer.

## Before a Full Review

Before starting a full detailed review, the reviewer should first evaluate whether the PR is in the right direction:

* A pull request should match our project principles. See [Development Principles](../contribute/principles.md).

* A pull request may partially address an issue, or be an incremental improvement rather than a full fix.

* A pull request does not need to be a complete or final solution, as long as it moves in the right direction and does not introduce regressions.

* A pull request should stay focused on a single, well-scoped change. If it grows to include unrelated changes along the way,
reviewers should ask the contributor to split it into separate PRs.

* If a pull request turns out to be a significant design change (e.g. a new API, a major refactor), reviewers should consider
escalating it for team discussion, and whether it should go through the [MEP process](../contribute/mep.md) before a
normal review.

A reviewer may reject a pull request without a detailed review, but must respectfully convey the reasons to the
contributor. The reasons may include, but are not limited to:
* The PR does not align with the project principles.
* The PR does not provide value.
* The PR brings in more downsides than value.

There might be further discussion, and the reviewer may change their mind.

If a reviewer finds the PR valuable, they should start a full review, and work collaboratively with the contributor
to eventually get the PR merged.

### Notes

A few other situations also affect whether a full review should start:

#### Failed CI Checks

Reviewers don't need to provide a full review if the PR has any failed CI checks. They may simply remind the
contributor of the failed check, especially for first-time contributors.

Reviewers may still review or approve a PR while CI is running, or failing for reasons unrelated to the PR (e.g. known flaky
tests).

#### Draft Pull Requests

A PR marked as draft/WIP does not need to be reviewed, unless explicitly asked for.
Reviewers may still leave early comments without being requested, but should bear in mind
that the PR is WIP and the author may already have intentions to change it.

## Details on Full Review

Ideally, a reviewer should be objective during the code review.

### Design

Design is highly subjective, and there might be multiple designs that work equally well for the same purpose.

The reviewer should not let personal preference bias the review.

When reviewing a design, the reviewer should justify their feedback with objective metrics where possible
(e.g. performance, complexity, maintainability).

### Correctness

We largely rely on automated tests for correctness. Code review should check if new code is covered by automated tests.

Code review does not have to focus on correctness. However, if the reviewer spots any correctness bug, they should
point it out, and ask the contributor to fix it (ideally with an automated regression test).

### Coding Style

We use standard Rust coding style, format, and lints. Our CI makes sure that PRs comply with the standards.

Some coding styles allowed by the CI may still reduce readability, or conflict with other goals in our project principles.
In these cases, reviewers may ask for changes, and at the same time, update the standards our CI checks for.
We should keep these exceptions to a minimum if possible.

Other than this, reviewers should allow various coding styles from the contributors.

### Performance

Performance is a core project principle. Reviewers may ask for a performance evaluation before approving a PR.

### Documentation

Clarity is also a core project principle. Reviewers may ask for documentation (especially doc comments, or in-code comments)
to be added, if they find the code hard to understand, or counter-intuitive.

## Leaving Reviews

Review comments should clearly state the reviewer's intention, so the contributor can act accordingly.

Reviewers may ask for clarification, request changes, provide soft suggestions, and more.

Comments should be resolved between reviewers and contributors.

### Disagreements

If a reviewer and a contributor cannot reach an agreement on a comment, either side should bring in a third team
member, or raise it in a team meeting, rather than leaving the PR blocked indefinitely.

## Approval and Merging

A PR needs at least one approval from a team member other than the author before it can be merged.

### Substantial Changes after Approval

If the contributor submits substantial changes after an approval, the changes require another round of review.
Any team member may revoke the existing approval to prevent the PR from being merged in the meantime.
