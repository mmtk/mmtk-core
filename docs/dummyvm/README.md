An example of MMTk/Rust-side Binding Implementation
===

A binding needs to implement certain Rust traits and may need to expose MMTk's Rust API to native code.
This Rust crate illustrates a minimal example of what needs to be implemented on the binding side in Rust.
When starting a new port of MMTk, developers can use this crate as a starting point by directly copying
it to their port. For more details, see [the porting guide](https://docs.mmtk.io/portingguide/howto/nogc.html#set-up).
