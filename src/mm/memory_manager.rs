use std::ptr::null_mut;
use libc::c_void;
use libc::c_char;

use std::ffi::CStr;
use std::str;

use std::sync::atomic::Ordering;

use plan::Plan;
use ::plan::MutatorContext;
use ::plan::TraceLocal;
use ::plan::CollectorContext;
use ::plan::ParallelCollectorGroup;

use ::vm::{Collection, VMCollection};

#[cfg(feature = "jikesrvm")]
use ::vm::jikesrvm::JTOC_BASE;

use ::util::{Address, ObjectReference};
use ::util::options::options::OptionMap;

use ::plan::selected_plan;
use self::selected_plan::SelectedPlan;

use ::plan::Allocator;

#[no_mangle]
#[cfg(feature = "jikesrvm")]
pub unsafe extern fn jikesrvm_gc_init(jtoc: *mut c_void, heap_size: usize) {
    ::util::logger::init().unwrap();
    JTOC_BASE = Address::from_mut_ptr(jtoc);
    selected_plan::PLAN.gc_init(heap_size);
    ::vm::JikesRVM::test1();
    info!("{}", ::vm::JikesRVM::test(44));
    info!("{}", ::vm::JikesRVM::test2(45, 67));
    info!("{}", ::vm::JikesRVM::test3(21, 34, 9, 8));
}

#[no_mangle]
#[cfg(not(feature = "jikesrvm"))]
pub extern fn jikesrvm_gc_init(_jtoc: *mut c_void, _heap_size: usize) {
    panic!("Cannot call jikesrvm_gc_init when not building for JikesRVM");
}

#[no_mangle]
#[cfg(feature = "jikesrvm")]
pub extern fn start_control_collector(thread_id: usize) {
    selected_plan::PLAN.control_collector_context.run(thread_id);
}

#[no_mangle]
#[cfg(not(feature = "jikesrvm"))]
pub extern fn start_control_collector(rvm_thread: *mut c_void) {
    panic!("Cannot call start_control_collector when not building for JikesRVM");
}

#[no_mangle]
pub unsafe extern fn gc_init(heap_size: usize) {
    if cfg!(feature = "jikesrvm") {
        panic!("Should be calling jikesrvm_gc_init instead");
    }
    ::util::logger::init().unwrap();
    selected_plan::PLAN.gc_init(heap_size);
    ::plan::plan::INITIALIZED.store(true, Ordering::SeqCst);
}

#[no_mangle]
pub extern fn bind_mutator(thread_id: usize) -> *mut c_void {
    SelectedPlan::bind_mutator(&selected_plan::PLAN, thread_id)
}

#[no_mangle]
pub unsafe fn alloc(mutator: *mut c_void, size: usize,
             align: usize, offset: isize, allocator: Allocator) -> *mut c_void {
    let local = &mut *(mutator as *mut <SelectedPlan as Plan>::MutatorT);
    local.alloc(size, align, offset, allocator).as_usize() as *mut c_void
}

#[no_mangle]
#[inline(never)]
pub unsafe fn alloc_slow(mutator: *mut c_void, size: usize,
                  align: usize, offset: isize, allocator: Allocator) -> *mut c_void {
    let local = &mut *(mutator as *mut <SelectedPlan as Plan>::MutatorT);
    local.alloc_slow(size, align, offset, allocator).as_usize() as *mut c_void
}

#[no_mangle]
pub unsafe extern fn mmtk_malloc(size: usize) -> *mut c_void {
    alloc(null_mut(), size, 1, 0, Allocator::Default)
}

#[no_mangle]
pub extern fn mmtk_free(_ptr: *const c_void) {}

#[no_mangle]
pub extern fn will_never_move(object: ObjectReference) -> bool {
    selected_plan::PLAN.will_never_move(object)
}

#[no_mangle]
pub unsafe extern fn report_delayed_root_edge(trace_local: *mut c_void, addr: *mut c_void) {
    trace!("JikesRVM called report_delayed_root_edge with trace_local={:?}", trace_local);
    let local = &mut *(trace_local as *mut <SelectedPlan as Plan>::TraceLocalT);
    local.report_delayed_root_edge(Address::from_usize(addr as usize));
    trace!("report_delayed_root_edge returned with trace_local={:?}", trace_local);
}

#[no_mangle]
pub unsafe extern fn will_not_move_in_current_collection(trace_local: *mut c_void, obj: *mut c_void) -> bool {
    trace!("will_not_move_in_current_collection({:?}, {:?})", trace_local, obj);
    let local = &mut *(trace_local as *mut <SelectedPlan as Plan>::TraceLocalT);
    let ret = local.will_not_move_in_current_collection(Address::from_usize(obj as usize).to_object_reference());
    trace!("will_not_move_in_current_collection returned with trace_local={:?}", trace_local);
    ret
}

#[no_mangle]
pub unsafe extern fn process_interior_edge(trace_local: *mut c_void, target: *mut c_void, slot: *mut c_void, root: bool) {
    trace!("JikesRVM called process_interior_edge with trace_local={:?}", trace_local);
    let local = &mut *(trace_local as *mut <SelectedPlan as Plan>::TraceLocalT);
    local.process_interior_edge(Address::from_usize(target as usize).to_object_reference(),
                                Address::from_usize(slot as usize), root);
    trace!("process_interior_root_edge returned with trace_local={:?}", trace_local);

}

#[no_mangle]
pub unsafe extern fn start_worker(thread_id: usize, worker: *mut c_void) {
    let worker_instance = &mut *(worker as *mut <SelectedPlan as Plan>::CollectorT);
    worker_instance.run(thread_id);
}

#[no_mangle]
#[cfg(feature = "jikesrvm")]
pub unsafe extern fn enable_collection(thread_id: usize) {
    (&mut *selected_plan::PLAN.control_collector_context.workers.get()).init_group(thread_id);
    VMCollection::spawn_worker_thread::<<SelectedPlan as Plan>::CollectorT>(thread_id, null_mut()); // spawn controller thread
    ::plan::plan::INITIALIZED.store(true, Ordering::SeqCst);
}

#[no_mangle]
#[cfg(not(feature = "jikesrvm"))]
pub extern fn enable_collection(size: usize) {
    panic!("Cannot call enable_collection when not building for JikesRVM");
}

#[no_mangle]
pub extern fn process(name: *const c_char, value: *const c_char) -> bool {
    let name_str: &CStr = unsafe { CStr::from_ptr(name) };
    let value_str: &CStr = unsafe { CStr::from_ptr(value) };
    let option = &OptionMap;
    unsafe {
        option.process(name_str.to_str().unwrap(), value_str.to_str().unwrap())
    }
}