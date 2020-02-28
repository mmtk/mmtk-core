extern crate mmtk;
extern crate libc;
#[macro_use]
extern crate lazy_static;

use std::ptr::null_mut;

use mmtk::vm::VMBinding;
use mmtk::util::OpaquePointer;
use mmtk::MMTK;
use mmtk::{VM_MAP, MMAPPER};
use mmtk::util::ObjectReference;
use mmtk::{Plan, SelectedPlan};
use libc::c_void;
pub mod scanning;
pub mod collection;
pub mod object_model;
pub mod active_plan;
pub mod reference_glue;
pub mod api;

#[repr(C)]
pub struct OpenJDK_Upcalls {
    pub stop_all_mutators: extern "C" fn(tls: OpaquePointer),
    pub resume_mutators: extern "C" fn(tls: OpaquePointer),
    pub spawn_collector_thread: extern "C" fn(tls: OpaquePointer, ctx: *mut <SelectedPlan<OpenJDK> as Plan<OpenJDK>>::CollectorT),
    pub block_for_gc: extern "C" fn(),
    pub active_collector: extern "C" fn(tls: OpaquePointer) -> *mut <SelectedPlan<OpenJDK> as Plan<OpenJDK>>::CollectorT,
    pub get_next_mutator: extern "C" fn() -> *mut <SelectedPlan<OpenJDK> as Plan<OpenJDK>>::MutatorT,
    pub reset_mutator_iterator: extern "C" fn(),
    pub compute_static_roots: extern "C" fn(trace: *mut c_void, tls: OpaquePointer),
    pub compute_global_roots: extern "C" fn(trace: *mut c_void, tls: OpaquePointer),
    pub compute_thread_roots: extern "C" fn(trace: *mut c_void, tls: OpaquePointer),
    pub scan_object: extern "C" fn(trace: *mut c_void, object: ObjectReference, tls: OpaquePointer),
    pub dump_object: extern "C" fn(object: ObjectReference),
    pub get_object_size: extern "C" fn(object: ObjectReference) -> usize,
    pub get_mmtk_mutator: extern "C" fn(tls: OpaquePointer) -> *mut <SelectedPlan<OpenJDK> as Plan<OpenJDK>>::MutatorT,
    pub is_mutator: extern "C" fn(tls: OpaquePointer) -> bool,
}

pub static mut UPCALLS: *const OpenJDK_Upcalls = null_mut();

pub struct OpenJDK;

impl VMBinding for OpenJDK {
    type VMObjectModel = object_model::VMObjectModel;
    type VMScanning = scanning::VMScanning;
    type VMCollection = collection::VMCollection;
    type VMActivePlan = active_plan::VMActivePlan;
    type VMReferenceGlue = reference_glue::VMReferenceGlue;
}

lazy_static! {
    pub static ref SINGLETON: MMTK<OpenJDK> = MMTK::new(&VM_MAP, &MMAPPER);
}
