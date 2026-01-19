//! This module holds common reference scanning policies used in MMTk core.

use crate::vm::RefScanPolicy;

#[allow(unused)] // For doc comments.
use crate::vm::{ObjectTracer,SlotVisitor};

/// An object is scanned during the strong transitive closure stage.  The VM binding should visit
/// fields that contain strong references using the [`SlotVisitor`] or [`ObjectTracer`] callbacks.
///
/// The VM binding should not visit weak reference fields using the [`SlotVisitor`] or
/// [`ObjectTracer`] callbacks.  If a VM binding chooses to discover weak references during tracing,
/// it should record relevant information (e.g. the current object, its fields, etc.) in VM-specific
/// data structures, as described in the [Porting Guide][pg-weakref].  If the VM binding chooses not
/// to discover weak reference fields this way, it can ignore weak fields.
///
/// [pg-weakref]:
///     https://docs.mmtk.io/portingguide/concerns/weakref.html#identifying-weak-references
pub struct StrongClosure;

impl RefScanPolicy for StrongClosure {
    const SHOULD_VISIT_STRONG: bool = true;
    const SHOULD_VISIT_WEAK: bool = false;
    const SHOULD_DISCOVER_WEAK: bool = true;
}

/// An object is scanned to update its references after objects are moved or after the new
/// addresses of objects have been calculated.  The VM binding should visit all reference fields
/// of an object, regardless whether they are holding strong or weak reference.
pub struct RefUpdate;

impl RefScanPolicy for RefUpdate {
    const SHOULD_VISIT_STRONG: bool = true;
    const SHOULD_VISIT_WEAK: bool = false;
    const SHOULD_DISCOVER_WEAK: bool = false;
}

/// Instruct the VM binding to visit all fields of an object, both strong and weak, without any
/// hints about the MMTk's intention to call the object-scanning function.
pub struct All;

impl RefScanPolicy for All {
    const SHOULD_VISIT_STRONG: bool = true;
    const SHOULD_VISIT_WEAK: bool = true;
    const SHOULD_DISCOVER_WEAK: bool = false;
}

/// Instruct the VM binding to visit all strong fields, without any hints about the MMTk's
/// intention to call the object-scanning function.  Particularly, the VM binding should not
/// discover weak references as [`StrongClosure`] implies.
pub struct StrongOnly;

impl RefScanPolicy for StrongOnly {
    const SHOULD_VISIT_STRONG: bool = true;
    const SHOULD_VISIT_WEAK: bool = false;
    const SHOULD_DISCOVER_WEAK: bool = false;
}

/// Instruct the VM binding to visit all weak fields, without any hints about the MMTk's
/// intention to call the object-scanning function.
pub struct WeakOnly;

impl RefScanPolicy for WeakOnly {
    const SHOULD_VISIT_STRONG: bool = false;
    const SHOULD_VISIT_WEAK: bool = true;
    const SHOULD_DISCOVER_WEAK: bool = false;
}
