//! MMTk-to-VM interface: the VMBinding trait.
//!
//! This module provides VM-specific traits that serve as MMTK-to-VM interfaces.
//! Each VM binding needs to provide an implementation for each of the traits.
//! MMTk requires the interfaces to be efficient, as some of the methods are called frequently
//! during collection (e.g. the methods for `ObjectModel`). We rely on cross-crate *link-time-optimization*
//! to remove the overhead of MMTk invoking methods on those traits.
//!
//! It is recommended for a VM binding that uses mmtk-core to do the following to ensure LTO is enabled for performance.
//! 1. Add the following section in the manifest file of a VM binding (`Cargo.toml`). This enables LTO for the release build:
//!    ```
//!    [profile.release]
//!    lto = true
//!    ```
//! 2. Make sure that the crate type for a VM binding supports LTO. To our knowledge, `staticlib` and `cdylib` support LTO, and
//! `rlib` does *not* support LTO.

/// Allows MMTk to access edges in a VM-defined way.
pub mod edge_shape;
mod finalizable;
pub(crate) mod metadata_specs;
/// Imports needed for VMBinding.
pub mod prelude;
mod scan_utils;
mod vmbinding;

pub use self::finalizable::Finalizable;
pub use self::metadata_specs::*;
pub use self::scan_utils::EdgeVisitor;
pub use self::scan_utils::ObjectTracer;
pub use self::scan_utils::ObjectTracerContext;
pub use self::scan_utils::RootsWorkFactory;
pub use self::vmbinding::GCThreadContext;
pub use self::vmbinding::VMBinding;
