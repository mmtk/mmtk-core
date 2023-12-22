---
name: MMTk Enhancememnt Proposal (MEP)
about: Use this template to create an MMTk Enhancement Proposal (MEP).
title: ''
labels: MEP
assignees: ''

---

# TL;DR

This section should use about one to three sentences to summarize the MEP.  As the name "TL;DR" (too
long, didn't read) suggests, this section should be short enough so that readers (including those in
a hurry) can get the main idea very quickly without reading through the MEP.

# Goal

What are the goals of the proposal?  This should be succinct.  If there's more than one goal,
enumerate them in a list.

-   goal 1
-   goal 2
-   ...

# Non-goal

Optional.  Use this section to explicitly exclusive goals out of the scope of the current MEP.

-   non-goal 1
-   non-goal 2
-   ...

# Success Metric

How do we determine whether the MEP is a success?  This can include improvements in performance,
safety, readability, maintainability, etc.  Enumerate the success metrics in a list (details should
be in the *Description* section).

# Motivation

Why should this work be done?  Who is asking for it?

Make sure the readers understand the problem this MEP is trying to address.  You can also mention
the languages, VMs, or users that need this enhancement.

If there are alternative ways to solve the problem, you can mention them here, but keep in mind that
there is an *Alternatives* section for adding more details.

# Description

This is the main section of the MEP.  Describe the enhancement in detail.

If you have already made prototype implementations for this MEP, add hyperlinks to the relevant PRs,
commits, repositories, etc.

If not, describe how you intend to implement this MEP.  You should think about all parts of
mmtk-core that your MEP may interact with.

This section will be long.  Feel free to add more subsections.

## Impact on Performance

Describe how this MEP will affect the performance.  "This MEP should not have any impact on performance" is still a valid description if it is so.

## Impact on Software Engineering

Describe whether this MEP will make software engineering easier or more difficult.  Will it make the code easier or harder to understand, maintain and/or change?

# Risks

In the following sub-sections, outline the *long-term* risks posed by this MEP and how those risks are mitigated.   **The core
purpose of the MEP process is to avoid the unintentional introduction of changes that bring
long-term negative impacts to MMTk**. This section is about identifying and accounting for risks
associated with such negative outcomes.  It should include the following subsections:

## Long Term Performance Risks

Enumerate possible negative long-term performance impacts of this MEP, and for each explain how that
risk will be mitigated.    Note: this is *not* about the immediate performance impact of the MEP,
but about the impact on future work.  For example, this includes identifying changes that may impede
the future introduction of entirely new algorithms, or entirely new optimizations.

It is OK for us to accept temporary minor performance reduction to make more significant
improvements possible.  On the contrary, it is bad to make changes to temporarily improve
performance and make long-term improvements hard or impossible.

## Long Term Software Engineering Risks

Enumerate possible negative long-term software engineering impacts of this MEP, and for each explain
how that risk will be mitigated.

One of the core goals of MMTk is making GC development easy.  If a developer wants to develop, for
example, a new GC algorithm, it should be easy to implement it quickly and easily in MMTk.  We don't
want changes that make this difficult.

## Impact on API

Outline how this MEP will affect APIs, both public and internal.   Enumerate the cases, and for each
case, explain how the risk of negative consequences will be mitigated and/or justify the change.

# Testing

If applicable, describe the way to reproduce the problem, and the way to check if the change
actually works.  For MMTk, it includes but is not limited to what (micro or macro) benchmarks to
use, and which VM binding (with or without changes) to use.

# Alternatives

Optional.  If there are more than one way to solve the problem, describe them here and explain why
this approach is preferred.

# Assumptions

Optional.  For the design changes of MMTk, this part mainly includes assumptions about, for example,
the VM's requirements, the GC algorithms we are supporting, the OS/architecture MMTK is running on,
etc.  If those assumptions no longer hold, we may need to reconsider the design.  Describe such
concerns in this section.

# Related Issues

Optional.  If there are related issues, PRs, etc., include them here.

Sometimes people create ordinary issues and PRs to fix some problems, and later MMTk developers
consider that the change is too fundamental and needs a more thorough reviewing process.  If that is
the case, add hyperlinks to those original issues and PRs.
