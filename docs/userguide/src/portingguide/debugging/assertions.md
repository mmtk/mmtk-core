# Enabling debug assertions

MMTk is implemented with an extensive amount of assertions to ensure the correctness.
We strongly recommend using a debug build of MMTk that includes all the debugging assertions
when one is developing on a MMTk binding. The assertions are normal Rust `debug_assert!`,
and they can be turned on in a release build with Rust flags (https://doc.rust-lang.org/cargo/reference/profiles.html#debug-assertions).

## Extreme debugging assertions

In addition to the normal debugging assertions, MMTk also has a set of
optional runtime checks that can be turned on by enabling the feature `extreme_assertions`.
These usually include checks that are too expensive (even in a debug build) that we do not
want to enable by default.

You should make sure your MMTk binding can pass all the assertions (including `extreme_assertions`).
