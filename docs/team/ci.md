# Continuous Integration

## Correctness Testing

MMTK core runs CI tests *before* a pull request is merged.

MMTk core sets up two sets of tests, the *minimal tests* and the *extended tests*.
* Minimal tests run unconditionally for every pull request and for every commit. This set of tests aims to finish within half an hour.
  This include tests from the `mmtk-core` repo and integration tests from binding repos. Integration tests with a binding in the minimal tests should
  focus on testing MMTk core features that is exclusively used for the binding.
* Extended tests only run for a pull request if the pull request is tagged with the label `PR-extended-testing`. This set of tests
  may take hours, and usually include integration tests with bindings which run the language implementation's standard test suite
  as much as possible.

## Performance Testing

We conduct performance testing for each MMTk core commit after it has been merged.

### Testing Environment and Epochs

We track the performance of MMTk over years. Naturally, changes in the testing environment (hardware or software) and methodology are sometimes necessary. Each time we make such a change, it marks the start of a new *epoch* in our performance evaluation.

Since changes in the testing environment can significantly impact performance, we do not directly compare performance results across different epochs. Within an epoch, we ensure that **MMTk does not experience performance regressions**, and **we only update the testing environment when there is no performance regression in the current epoch**.

### Regression Test Canary

To monitor unnoticed performance changes and to measure the level of noise in the testing environment, we use a canary when doing performance regression tests with the OpenJDK binding.  A "canary" is a chosen revision that is run along with any merged pull request.  Since the same revision is run again and again, its performance should be relatively constant, within the range of noise.  If we notice a change in the performance of the canary (especially something that resembles a [step function](https://en.wikipedia.org/wiki/Heaviside_step_function) in the line plot), we should inspect our testing environment for hardware or software changes.

We keep running the same canary version until it is no longer possible, for reasons such as the toolchain for compiling that version is no longer available.  When that happens, we may choose a different canary version or switch to an automatic mechanism for choosing canary.
