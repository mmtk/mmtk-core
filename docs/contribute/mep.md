MMTk Enhancement Proposal (MEP)
===============================

An MMTk Enhancement Proposal (MEP) is a formal process for the MMTk team and its developers to
propose significant design changes, review the impact of the changes and make informed decisions.
It has a special format, and will undergo a more thorough review process.  Its goal is helping the
MMTk developers making more informed decisions.

MEP is inspired by the Java Enhancement Proposal, described in https://openjdk.org/jeps/1

# When is MEP required?

An MEP is required when making a significant change to the design of the MMTk core.  It is
applicable to any kind of significant changes, including but not limited to:

-   Bug fixes, including performance bug fixes, that require change to a major part of the MMTk
    core.
-   Changes to the MMTk core to implement a feature demanded by bindings.
-   Major refactoring to the MMTk core.

**The core purpose of the MEP process is to avoid the unintentional introduction of changes that
bring long-term negative impacts to MMTk.**  Large-scale changes and public API changes usually
indicate such risks, but these are only indicators, not criteria.  The assessment of risks is mostly
subjective, and the MMTk team need to discuss in order to reach consensus.

If a contributor is uncertain if they should submit an MEP for their proposed changes, we encourage
them to talk with the MMTk team first, or to simply submit a normal PR/issue to get it started. An
MEP would be requested by the MMTk team if necessary (See the details about this in the section of
MEP review process).

# Format

A MEP will be posted as a GitHub issue in the `mmtk-core` repository.  It should contain `MEP` tag.

A MEP should have the following sections:

-   TL;DR
-   Goal
-   Non-goal (optional)
-   Success Metric
-   Motivation
-   Description
    -   Impact on Performance
    -   Impact on Software Engineering
-   Risks
    -   Long Term Performance Risks
    -   Long Term Software Engineering Risks
    -   Impact on API
-   Testing
-   Alternatives (optional)
-   Risks and Assumptions (optional)
-   Related Issues (optional)

# Sections

## TL;DR

This section should use about one to three sentences to summarize the MEP.  As the name "TL;DR" (too
long, didn't read) suggests, this section should be short enough so that readers (including those in
a hurry) can get the main idea very quickly without reading through the MEP.

## Goals

What are the goals of the proposal?  This should be succinct.  If there's more than one goal,
enumerate them in a list.

## Non-Goals

Optional.  Use this section to explicitly exclusive goals out of the scope of the current MEP.

## Success Metric

How do we determine whether the MEP is a success?  This can include improvements in performance,
safety, readability, maintainability, etc.  Enumerate the success metrics in a list (details should
be in the *Description* section).

## Motivation

Why should this work be done?  Who is asking for it?

Make sure the readers understand the problem this MEP is trying to address.  You can also mention
the languages, VMs, or users that need this enhancement.

If there are alternative ways to solve the problem, you can mention them here, but keep in mind that
there is an *Alternatives* section for adding more details.

## Description

This is the main section of the MEP.  Describe the enhancement in detail.

If you have already made prototype implementations for this MEP, add hyperlinks to the relevant PRs,
commits, repositories, etc.

If not, describe how you intend to implement this MEP.  You should think about all parts of
mmtk-core that your MEP may interact with.

This section will be long, and will usually be divided into many subsections.  The following
subsections must be included:

-   Impact on Performance
-   Impact on Software Engineering

### Impact on Performance

Describe how this MEP will affect the performance.  "This MEP should not have any impact on
performance" is still a valid description if it is so.

### Impact on Software Engineering

Describe whether this MEP will make software engineering easier or more difficult.  Will it make the
code easier or harder to understand, maintain and/or change?

## Risks

Outline the *long-term* risks posed by this MEP and how those risks are mitigated.   **The core
purpose of the MEP process is to avoid the unintentional introduction of changes that bring
long-term negative impacts to MMTk**. This section is about identifying and accounting for risks
associated with such negative outcomes.  It should include the following subsections:

-   Long Term Performance Risks
-   Long Term Software Engineering Risks
-   Impact on API

### Long Term Performance Risks

Enumerate possible negative long-term performance impacts of this MEP, and for each explain how that
risk will be mitigated.    Note: this is *not* about the immediate performance impact of the MEP,
but about the impact on future work.  For example, this includes identifying changes that may impede
the future introduction of entirely new algorithms, or entirely new optimizations.

It is OK for us to accept temporary minor performance reduction to make more significant
improvements possible.  On the contrary, it is bad to make changes to temporarily improve
performance and make long-term improvements hard or impossible.

### Long Term Software Engineering Risks

Enumerate possible negative long-term software engineering impacts of this MEP, and for each explain
how that risk will be mitigated.

One of the core goals of MMTk is making GC development easy.  If a developer wants to develop, for
example, a new GC algorithm, it should be easy to implement it quickly and easily in MMTk.  We don't
want changes that make this difficult.

### Impact on API

Outline how this MEP will affect APIs, both public and internal.   Enumerate the cases, and for each
case, explain how the risk of negative consequences will be mitigated and/or justify the change.

## Testing

If applicable, describe the way to reproduce the problem, and the way to check if the change
actually works.  For MMTk, it includes but is not limited to what (micro or macro) benchmarks to
use, and which VM binding (with or without changes) to use.

## Alternatives

Optional.  If there are more than one way to solve the problem, describe them here and explain why
this approach is preferred.

## Assumptions

Optional.  For the design changes of MMTk, this part mainly includes assumptions about, for example,
the VM's requirements, the GC algorithms we are supporting, the OS/architecture MMTK is running on,
etc.  If those assumptions no longer hold, we may need to reconsider the design.  Describe such
concerns in this section.

## Related Issues

Optional.  If there are related issues, PRs, etc., include them here.

Sometimes people create ordinary issues and PRs to fix some problems, and later MMTk developers
consider that the change is too fundamental and needs a more thorough reviewing process.  If that is
the case, add hyperlinks to those original issues and PRs.

# MEP Reviewing Process

## Initiate an MEP

An MEP is initiated by creating an MEP issue with the format described in this document.

### Escalate normal PRs/issues and request for MEP

For normal PRs/issues, if any team member thinks it should be an MEP, they should escalate it and
discuss with the team. If the team decide that it should be an MEP, the PR/issue should be set to
'Request for MEP', and expects an MEP issue from the contributor. If there is no further action from
the contributor for a period of time, the PR/issue is considered as stale and it may get assigned to
an MMTk team member or may be closed.

Note that a team member may escalate an PR/issue for normal discussion, rather than requesting for
MEP. As we discussed, MEP is a heavy-weight process, and we should not abuse requesting it.

## Review an MEP

### Criteria

The MMTk team will first decide if the proposal meets the criteria for being an MEP. If it does not
meet the criteria, the proposal issue will be closed, and related changes should be treated as a
normal PR/issue.

The criteria for what need to be an MEP is mostly subjective, based on a consensus model within the
MMTk team. We also provide a list of exemption for what do not need to be an MEP.

#### MEP Exemption

MEP is intended to help avoid design changes that may have profound negative impact in the future.
Some changes will not have profound impact, and can be easily reverted if necessary. They should be
exempt from the heavy-weight MEP process, and should not be escalated to request for MEP. The
exemption is intended to ensure that we won't abuse using MEP and that we won't impose burden on the
contributors to submit an extra MEP proposal. An exempted PR may still be escalated for team
discussion, but it is exempt from being requested for MEP (submitting a MEP proposal, and going
through the MEP process).

##### Exemption 1: Well-encapsulated changes

Changes that are well-encapsulated and decoupled intrinsically can be easily corrected in the future
and will not have profound impact for the future. A PR that has no public API change, and no module
API change between the top-level modules (`plan`, `policy`, `scheduler`, `util` and `vm` at the time
of writing) is exempt from MEP.

### Review

The MMTk team will discuss the proposal in weekly meetings. This process may take a while. We will
keep posting the discussion to the MEP issue, and encourage further inputs from the contributor, and
the community. An MEP may get updated and refined during the process.

### Outcome

At the end of the review, the MEP will be accepted or rejected based on the consensus of the MMTk
team. If an MEP is accepted, A PR may follow the MEP and will be reviewed with the normal PR review
process.

If an MEP is rejected, future related MEPs may not be reviewed again unless they are substantially
different. We encourage people to get involved in the review discussion, and refine the proposal so
it will be accepted.

## Communication

After an MEP is implemented, the MMTk team shall announce the significant changes as the results of
the MEP to make them known to the community.  We shall communicate with our sponsors about such
changes, too, in our regular meetings.

<!--
vim: ts=4 sw=4 sts=4 et tw=100
-->
