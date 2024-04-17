// Some mock methods may get really complex
#![allow(clippy::type_complexity)]

use crate::plan::ObjectQueue;
use crate::scheduler::gc_work::ProcessEdgesWorkRootsWorkFactory;
use crate::scheduler::gc_work::ProcessEdgesWorkTracerContext;
use crate::scheduler::gc_work::SFTProcessEdges;
use crate::scheduler::*;
use crate::util::alloc::AllocationError;
use crate::util::copy::*;
use crate::util::heap::gc_trigger::GCTriggerPolicy;
use crate::util::opaque_pointer::*;
use crate::util::{Address, ObjectReference};
use crate::vm::object_model::specs::*;
use crate::vm::EdgeVisitor;
use crate::vm::GCThreadContext;
use crate::vm::ObjectTracer;
use crate::vm::ObjectTracerContext;
use crate::vm::RootsWorkFactory;
use crate::vm::VMBinding;
use crate::Mutator;

use super::mock_method::*;

use std::default::Default;
use std::ops::Range;
use std::sync::Mutex;

/// The offset between object reference and the allocation address if we use
/// the default mock VM.
pub const DEFAULT_OBJECT_REF_OFFSET: usize = 4;

// To mock static methods, we have to create a static instance of `MockVM`.
lazy_static! {
    // The mutex may get poisoned any time. Accessing this mutex needs to deal with the poisoned case.
    // One can use read/write_mockvm to access mock vm.
    static ref MOCK_VM_INSTANCE: Mutex<MockVM> = Mutex::new(MockVM::default());
}

// MockVM only allows mock methods with references of no lifetime or static lifetime.
// If `VMBinding` methods has references of a specific lifetime,
// the references need to be turned into static lifetime before we can call mock methods.
// This is correct as long as we only use the references within the mock methods, and
// we do not store them for access after the mock method returns.
macro_rules! lifetime {
    ($e: expr) => {
        unsafe { std::mem::transmute($e) }
    };
}

/// Call `MockMethod`.
macro_rules! mock {
    ($fn: ident($($arg:expr),*)) => {
        write_mockvm(|mock| mock.$fn.call(($($arg),*)))
    };
}
/// Call `MockAny`.
macro_rules! mock_any {
    ($fn: ident($($arg:expr),*)) => {
        *write_mockvm(|mock| mock.$fn.call_any(Box::new(($($arg),*)))).downcast().unwrap()
    };
}

/// Read from the static MockVM instance. It deals with the case of a poisoned lock.
pub fn read_mockvm<F, R>(func: F) -> R
where
    F: FnOnce(&MockVM) -> R,
{
    let lock = MOCK_VM_INSTANCE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    func(&lock)
}
/// Write to the static MockVM instance. It deals with the case of a poisoned lock.
pub fn write_mockvm<F, R>(func: F) -> R
where
    F: FnOnce(&mut MockVM) -> R,
{
    let mut lock = MOCK_VM_INSTANCE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    func(&mut lock)
}

/// A test that uses `MockVM` should use this method to wrap the entire test
/// that may use `MockVM`.
///
/// # Arguents
/// * `setup`: Create a `MockVM`. Most tests can just use the default `MockVM::default()`.
///   A test may also overwrite some methods for its own testing purpose.
/// * `test`: The actual test. All the code that may access `MockVM`/`VMBinding` should be
///   wrapped in the test closure.
/// * `cleanup`: Any clean up or post check when the test finishes or aborts.
pub fn with_mockvm<S, T, C>(setup: S, test: T, cleanup: C)
where
    S: FnOnce() -> MockVM,
    T: FnOnce() + std::panic::UnwindSafe,
    C: FnOnce(),
{
    super::serial_test(|| {
        // Setup
        {
            write_mockvm(|mock| *mock = setup());
        }
        super::with_cleanup(test, cleanup);
    })
}

/// Set up a default `MockVM`
pub fn default_setup() -> MockVM {
    MockVM::default()
}

/// No extra clean up after the test.
pub fn no_cleanup() {}

/// A struct that allows us to mock the behavior of a `VMBinding` and the VM traits for testing.
/// For simplicity, we implement `VMBinding` as well as `ActivePlan`, `Collection`,
/// `ObjectModel`, `ReferenceGlue`, `Scanning` on the `MockVM` type, and forward each
/// method to the mock methods in the static `MockVM` instance.
/// By changing the mock closures in the struct, we can control the behavior of the `VMBinding`.
/// Use [`with_mockvm`] in the tests that need `MockVM`.
///
/// # Mocking methods
///
/// The struct includes one mock method for each methods in the VM traits.
///
/// ## Methods with only value types
///
/// It is straighforward to mock methods with only value types. Just group the argument types,
/// and the return types into two tuples (e.g. `I` and `R`), and create a `MockMethod<I, R>`.
/// For example, [`crate::vm::ActivePlan::is_mutator`] has a signature of `fn(VMThread) -> bool`,
/// we can just create `MockMethod<VMThread, bool>` for it.
///
/// ## Methods with reference types
///
/// As we cannot have extra type parameters (including generic lifetime paraeters) on `MockVM`, `MockVM` can only
/// have `MockMethod` with types of `'static` lifetime. To create a mock method for methods with
/// reference types, just replace the lifetime specifier in the reference with `'static` lifetime.
/// For example, [`crate::vm::ActivePlan::mutators`] has a signature of `fn<'a>() -> Box<dyn Iterator<Item = &'a mut Mutator<VM>> + 'a>`,
/// we just replace all the lifetime specifiers with `'static.`, and create
/// `MockMethod<(), Box<dyn Iterator<Item = &'static mut Mutator<MockVM>> + 'static>>`.
/// When we invoke the `MockMethod`, we can use the `lifetime!` macro to hack the lifetime.
/// Though this is unsafe, it is correct as long as we only use the reference within the mock implementation.
///
/// ## Methods with generic type parameters
///
/// As we cannot have extra type parameters on `MockVM`, there are two ways
/// to mock methods with generic type parameters.
///
/// ### Use trait objects
///
/// We can use trait objects if the trait is object safe. For example,
/// [`crate::vm::ActivePlan::vm_trace_object`] has a signature of
/// `fn<Q: ObjectQueue>(&mut Q, ObjectReference, &mut GCWorker<VM>) -> ObjectReference`,
/// we can mock `&mut Q` as `&mut dyn ObjectQueue`, and use
/// `MockMethod<(&'static mut dyn ObjectQueue, ObjectReference, &'static mut GCWorker<MockVM>), ObjectReference>`
/// for the method.
///
/// ### Use `MockAny`
///
/// For cases where we cannot use trait objects, we can use `MockAny`.
/// We simply use `Box<MockAny>` and initiate it with a `MockMethod` of
/// concrete types. For example, [`crate::vm::Scanning::process_weak_refs`]
/// has a signature of `fn(&mut GCWorker<VM>, impl ObjectTracerContext<VM>`.
/// `ObjectTracerContext` is not object safe. So we just use `Box<MockAny>`
/// in `MockVM`, and initiate it with a concrete type of `ObjectTracerContext`, such as
/// `Box::new((MockMethod::<(&'static mut GCWorker<Self>,ProcessEdgesWorkTracerContext<SFTProcessEdges<Self>>,),bool>::new_unimplemented())`.
///
/// Note that when `MockAny` is used, one needs to make sure that the types of the actual arguments match the argument types used for creating the `MockMethod`.
/// We provide a default implementation for those `MockAny` methods, and it is very possible that the types in the default implementation do not
/// match the arguments you would like to test with. You should overwrite the default `MockMethod` during the MockVM setup.
///
/// # Mock constants and associated types
///
/// These are not supported at the moment. As those will change the `MockVM` type, we will have
/// to use macros to generate a new `MockVM` type when we custimize constants or associated types.

// The current implementation is not perfect, but at least it works, and it is easy enough to debug with.
// I have tried different third-party libraries for mocking, and each has its own limitation. And
// none of the libraries I tried can mock `VMBinding` and the associated traits out of box. Even after I attempted
// to remove all those VM traits and had all the methods in `VMBinding`, the libraries still did not
// work out.
pub struct MockVM {
    // active plan
    pub number_of_mutators: MockMethod<(), usize>,
    pub is_mutator: MockMethod<VMThread, bool>,
    pub mutator: MockMethod<VMMutatorThread, &'static mut Mutator<MockVM>>,
    pub mutators: MockMethod<(), Box<dyn Iterator<Item = &'static mut Mutator<MockVM>> + 'static>>,
    pub vm_trace_object: MockMethod<
        (
            &'static mut dyn ObjectQueue,
            ObjectReference,
            &'static mut GCWorker<MockVM>,
        ),
        ObjectReference,
    >,
    // collection
    pub stop_all_mutators:
        MockMethod<(VMWorkerThread, Box<dyn FnMut(&'static mut Mutator<MockVM>)>), ()>,
    pub resume_mutators: MockMethod<VMWorkerThread, ()>,
    pub block_for_gc: MockMethod<VMMutatorThread, ()>,
    pub spawn_gc_thread: MockMethod<(VMThread, GCThreadContext<MockVM>), ()>,
    pub out_of_memory: MockMethod<(VMThread, AllocationError), ()>,
    pub schedule_finalization: MockMethod<VMWorkerThread, ()>,
    pub post_forwarding: MockMethod<VMWorkerThread, ()>,
    pub vm_live_bytes: MockMethod<(), usize>,
    pub is_collection_enabled: MockMethod<(), bool>,
    pub create_gc_trigger: MockMethod<(), Box<dyn GCTriggerPolicy<MockVM>>>,
    // object model
    pub copy_object: MockMethod<
        (
            ObjectReference,
            CopySemantics,
            &'static GCWorkerCopyContext<MockVM>,
        ),
        ObjectReference,
    >,
    pub copy_object_to: MockMethod<(ObjectReference, ObjectReference, Address), Address>,
    pub get_object_size: MockMethod<ObjectReference, usize>,
    pub get_object_size_when_copied: MockMethod<ObjectReference, usize>,
    pub get_object_align_when_copied: MockMethod<ObjectReference, usize>,
    pub get_object_align_offset_when_copied: MockMethod<ObjectReference, usize>,
    pub get_type_descriptor: MockMethod<(), &'static [i8]>,
    pub get_object_reference_when_copied_to:
        MockMethod<(ObjectReference, Address), ObjectReference>,
    pub ref_to_object_start: MockMethod<ObjectReference, Address>,
    pub ref_to_header: MockMethod<ObjectReference, Address>,
    pub ref_to_address: MockMethod<ObjectReference, Address>,
    pub address_to_ref: MockMethod<Address, ObjectReference>,
    pub dump_object: MockMethod<ObjectReference, ()>,
    // reference glue
    pub weakref_clear_referent: MockMethod<ObjectReference, ()>,
    pub weakref_set_referent: MockMethod<(ObjectReference, ObjectReference), ()>,
    pub weakref_get_referent: MockMethod<ObjectReference, ObjectReference>,
    pub weakref_is_referent_cleared: MockMethod<ObjectReference, bool>,
    pub weakref_enqueue_references: MockMethod<(&'static [ObjectReference], VMWorkerThread), ()>,
    // scanning
    pub support_edge_enqueuing: MockMethod<(VMWorkerThread, ObjectReference), bool>,
    pub scan_object: MockMethod<
        (
            VMWorkerThread,
            ObjectReference,
            &'static mut dyn EdgeVisitor<<MockVM as VMBinding>::VMEdge>,
        ),
        (),
    >,
    pub scan_object_and_trace_edges: MockMethod<
        (
            VMWorkerThread,
            ObjectReference,
            &'static mut dyn ObjectTracer,
        ),
        (),
    >,
    pub scan_roots_in_mutator_thread: Box<dyn MockAny>,
    pub scan_vm_specific_roots: Box<dyn MockAny>,
    pub notify_initial_thread_scan_complete: MockMethod<(bool, VMWorkerThread), ()>,
    pub supports_return_barrier: MockMethod<(), bool>,
    pub prepare_for_roots_re_scanning: MockMethod<(), ()>,
    pub process_weak_refs: Box<dyn MockAny>,
    pub forward_weak_refs: Box<dyn MockAny>,
}

impl Default for MockVM {
    fn default() -> Self {
        Self {
            number_of_mutators: MockMethod::new_unimplemented(),
            is_mutator: MockMethod::new_fixed(Box::new(|_| true)),
            mutator: MockMethod::new_unimplemented(),
            mutators: MockMethod::new_unimplemented(),
            vm_trace_object: MockMethod::new_fixed(Box::new(|(_, object, _)| {
                panic!("MMTk cannot trace object {:?} as it does not belong to any MMTk space. If the object is known to the VM, the binding can override this method and handle its tracing.", object)
            })),

            stop_all_mutators: MockMethod::new_unimplemented(),
            resume_mutators: MockMethod::new_unimplemented(),
            block_for_gc: MockMethod::new_unimplemented(),
            spawn_gc_thread: MockMethod::new_default(),
            out_of_memory: MockMethod::new_fixed(Box::new(|(_, err)| {
                panic!("Out of memory with {:?}!", err)
            })),
            schedule_finalization: MockMethod::new_default(),
            post_forwarding: MockMethod::new_default(),
            vm_live_bytes: MockMethod::new_default(),
            is_collection_enabled: MockMethod::new_fixed(Box::new(|_| true)),
            create_gc_trigger: MockMethod::new_unimplemented(),

            copy_object: MockMethod::new_unimplemented(),
            copy_object_to: MockMethod::new_unimplemented(),
            get_object_size: MockMethod::new_unimplemented(),
            get_object_size_when_copied: MockMethod::new_unimplemented(),
            get_object_align_when_copied: MockMethod::new_fixed(Box::new(|_| {
                std::mem::size_of::<usize>()
            })),
            get_object_align_offset_when_copied: MockMethod::new_fixed(Box::new(|_| 0)),
            get_type_descriptor: MockMethod::new_unimplemented(),
            get_object_reference_when_copied_to: MockMethod::new_unimplemented(),
            ref_to_object_start: MockMethod::new_fixed(Box::new(|object| {
                object.to_raw_address().sub(DEFAULT_OBJECT_REF_OFFSET)
            })),
            ref_to_header: MockMethod::new_fixed(Box::new(|object| object.to_raw_address())),
            ref_to_address: MockMethod::new_fixed(Box::new(|object| {
                object.to_raw_address().sub(DEFAULT_OBJECT_REF_OFFSET)
            })),
            address_to_ref: MockMethod::new_fixed(Box::new(|addr| {
                ObjectReference::from_raw_address(addr.add(DEFAULT_OBJECT_REF_OFFSET))
            })),
            dump_object: MockMethod::new_unimplemented(),

            weakref_clear_referent: MockMethod::new_unimplemented(),
            weakref_get_referent: MockMethod::new_unimplemented(),
            weakref_set_referent: MockMethod::new_unimplemented(),
            weakref_is_referent_cleared: MockMethod::new_fixed(Box::new(|r| r.is_null())),
            weakref_enqueue_references: MockMethod::new_unimplemented(),

            support_edge_enqueuing: MockMethod::new_fixed(Box::new(|_| true)),
            scan_object: MockMethod::new_unimplemented(),
            scan_object_and_trace_edges: MockMethod::new_unimplemented(),
            // We instantiate a `MockMethod` with the arguments as ProcessEdgesWorkRootsWorkFactory<..., SFTProcessEdges<MockVM>, ...>,
            // thus the mock method expects the actual call arguments to match the type.
            // In most cases, this won't work and this `MockMethod` is just a place holder. It is
            // fine as long as the method is not actually called.
            // If the user will need this method, and would like to mock the method in their particular test,
            // they are expected to provide their own
            // `MockMethod` that matches the argument types they will pass for the test case.
            // See the documents on the section about `MockAny` on the `MockVM` type.
            scan_roots_in_mutator_thread: Box::new(MockMethod::<
                (
                    VMWorkerThread,
                    &'static mut Mutator<MockVM>,
                    ProcessEdgesWorkRootsWorkFactory<
                        MockVM,
                        SFTProcessEdges<MockVM>,
                        SFTProcessEdges<MockVM>,
                    >,
                ),
                (),
            >::new_unimplemented()),
            // Same here: the `MockMethod` is just a place holder. See the above comments.
            scan_vm_specific_roots: Box::new(MockMethod::<
                (
                    VMWorkerThread,
                    ProcessEdgesWorkRootsWorkFactory<
                        MockVM,
                        SFTProcessEdges<MockVM>,
                        SFTProcessEdges<MockVM>,
                    >,
                ),
                (),
            >::new_unimplemented()),
            notify_initial_thread_scan_complete: MockMethod::new_unimplemented(),
            supports_return_barrier: MockMethod::new_unimplemented(),
            prepare_for_roots_re_scanning: MockMethod::new_unimplemented(),
            // Same here: the `MockMethod` is just a place holder. See the above comments.
            process_weak_refs: Box::new(MockMethod::<
                (
                    &'static mut GCWorker<Self>,
                    ProcessEdgesWorkTracerContext<SFTProcessEdges<MockVM>>,
                ),
                bool,
            >::new_unimplemented()),
            // Same here: the `MockMethod` is just a place holder. See the above comments.
            forward_weak_refs: Box::new(MockMethod::<
                (
                    &'static mut GCWorker<Self>,
                    ProcessEdgesWorkTracerContext<SFTProcessEdges<MockVM>>,
                ),
                (),
            >::new_default()),
        }
    }
}

unsafe impl Sync for MockVM {}
unsafe impl Send for MockVM {}

impl VMBinding for MockVM {
    type VMEdge = Address;
    type VMMemorySlice = Range<Address>;

    type VMActivePlan = MockVM;
    type VMCollection = MockVM;
    type VMObjectModel = MockVM;
    type VMReferenceGlue = MockVM;
    type VMScanning = MockVM;

    /// Allowed maximum alignment in bytes.
    const MAX_ALIGNMENT: usize = 1 << 6;
}

impl crate::vm::ActivePlan<MockVM> for MockVM {
    fn number_of_mutators() -> usize {
        mock!(number_of_mutators())
    }

    fn is_mutator(tls: VMThread) -> bool {
        mock!(is_mutator(tls))
    }

    fn mutator(tls: VMMutatorThread) -> &'static mut Mutator<MockVM> {
        mock!(mutator(tls))
    }

    fn mutators<'a>() -> Box<dyn Iterator<Item = &'a mut Mutator<MockVM>> + 'a> {
        let ret = mock!(mutators());
        lifetime!(ret)
    }

    fn vm_trace_object<Q: ObjectQueue>(
        queue: &mut Q,
        object: ObjectReference,
        worker: &mut GCWorker<MockVM>,
    ) -> ObjectReference {
        mock!(vm_trace_object(
            lifetime!(queue as &mut dyn ObjectQueue),
            object,
            lifetime!(worker)
        ))
    }
}

impl crate::vm::Collection<MockVM> for MockVM {
    fn stop_all_mutators<F>(tls: VMWorkerThread, mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<MockVM>),
    {
        mock!(stop_all_mutators(
            tls,
            lifetime!(Box::new(mutator_visitor) as Box<dyn FnMut(&'static mut Mutator<MockVM>)>)
        ))
    }

    fn resume_mutators(tls: VMWorkerThread) {
        mock!(resume_mutators(tls))
    }

    fn block_for_gc(tls: VMMutatorThread) {
        mock!(block_for_gc(tls))
    }

    fn spawn_gc_thread(tls: VMThread, ctx: GCThreadContext<MockVM>) {
        mock!(spawn_gc_thread(tls, ctx))
    }

    fn out_of_memory(tls: VMThread, err_kind: AllocationError) {
        mock!(out_of_memory(tls, err_kind))
    }

    fn schedule_finalization(tls: VMWorkerThread) {
        mock!(schedule_finalization(tls))
    }

    fn post_forwarding(tls: VMWorkerThread) {
        mock!(post_forwarding(tls))
    }

    fn is_collection_enabled() -> bool {
        mock!(is_collection_enabled())
    }

    fn vm_live_bytes() -> usize {
        mock!(vm_live_bytes())
    }

    fn create_gc_trigger() -> Box<dyn GCTriggerPolicy<MockVM>> {
        mock!(create_gc_trigger())
    }
}

impl crate::vm::ObjectModel<MockVM> for MockVM {
    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec = VMGlobalLogBitSpec::in_header(0);
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec =
        VMLocalForwardingPointerSpec::in_header(0);
    const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec =
        VMLocalForwardingBitsSpec::in_header(0);
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec = VMLocalMarkBitSpec::in_header(0);
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec =
        VMLocalLOSMarkNurserySpec::in_header(0);

    #[cfg(feature = "object_pinning")]
    const LOCAL_PINNING_BIT_SPEC: VMLocalPinningBitSpec = VMLocalPinningBitSpec::in_header(0);

    const OBJECT_REF_OFFSET_LOWER_BOUND: isize = DEFAULT_OBJECT_REF_OFFSET as isize;

    fn copy(
        from: ObjectReference,
        semantics: CopySemantics,
        copy_context: &mut GCWorkerCopyContext<MockVM>,
    ) -> ObjectReference {
        mock!(copy_object(from, semantics, lifetime!(copy_context)))
    }

    fn copy_to(from: ObjectReference, to: ObjectReference, region: Address) -> Address {
        mock!(copy_object_to(from, to, region))
    }

    fn get_current_size(object: ObjectReference) -> usize {
        mock!(get_object_size(object))
    }

    fn get_size_when_copied(object: ObjectReference) -> usize {
        mock!(get_object_size_when_copied(object))
    }

    fn get_align_when_copied(object: ObjectReference) -> usize {
        mock!(get_object_align_when_copied(object))
    }

    fn get_align_offset_when_copied(object: ObjectReference) -> usize {
        mock!(get_object_align_offset_when_copied(object))
    }

    fn get_type_descriptor(_reference: ObjectReference) -> &'static [i8] {
        // We do not use this method, and it will be removed.
        unreachable!()
    }

    fn get_reference_when_copied_to(from: ObjectReference, to: Address) -> ObjectReference {
        mock!(get_object_reference_when_copied_to(from, to))
    }

    fn ref_to_object_start(object: ObjectReference) -> Address {
        mock!(ref_to_object_start(object))
    }

    fn ref_to_header(object: ObjectReference) -> Address {
        mock!(ref_to_header(object))
    }

    fn ref_to_address(object: ObjectReference) -> Address {
        mock!(ref_to_address(object))
    }

    fn address_to_ref(addr: Address) -> ObjectReference {
        mock!(address_to_ref(addr))
    }

    fn dump_object(object: ObjectReference) {
        mock!(dump_object(object))
    }
}

impl crate::vm::ReferenceGlue<MockVM> for MockVM {
    type FinalizableType = ObjectReference;

    fn clear_referent(new_reference: ObjectReference) {
        mock!(weakref_clear_referent(new_reference))
    }

    fn set_referent(reference: ObjectReference, referent: ObjectReference) {
        mock!(weakref_set_referent(reference, referent))
    }
    fn get_referent(object: ObjectReference) -> ObjectReference {
        mock!(weakref_get_referent(object))
    }
    fn is_referent_cleared(referent: ObjectReference) -> bool {
        mock!(weakref_is_referent_cleared(referent))
    }
    fn enqueue_references(references: &[ObjectReference], tls: VMWorkerThread) {
        mock!(weakref_enqueue_references(lifetime!(references), tls))
    }
}

impl crate::vm::Scanning<MockVM> for MockVM {
    fn support_edge_enqueuing(tls: VMWorkerThread, object: ObjectReference) -> bool {
        mock!(support_edge_enqueuing(tls, object))
    }
    fn scan_object<EV: EdgeVisitor<<MockVM as VMBinding>::VMEdge>>(
        tls: VMWorkerThread,
        object: ObjectReference,
        edge_visitor: &mut EV,
    ) {
        mock!(scan_object(
            tls,
            object,
            lifetime!(edge_visitor as &mut dyn EdgeVisitor<<MockVM as VMBinding>::VMEdge>)
        ))
    }
    fn scan_object_and_trace_edges<OT: ObjectTracer>(
        tls: VMWorkerThread,
        object: ObjectReference,
        object_tracer: &mut OT,
    ) {
        mock!(scan_object_and_trace_edges(
            tls,
            object,
            lifetime!(object_tracer as &mut dyn ObjectTracer)
        ))
    }
    fn scan_roots_in_mutator_thread(
        tls: VMWorkerThread,
        mutator: &'static mut Mutator<Self>,
        factory: impl RootsWorkFactory<<MockVM as VMBinding>::VMEdge>,
    ) {
        mock_any!(scan_roots_in_mutator_thread(
            tls,
            mutator,
            Box::new(factory)
        ))
    }
    fn scan_vm_specific_roots(
        tls: VMWorkerThread,
        factory: impl RootsWorkFactory<<MockVM as VMBinding>::VMEdge>,
    ) {
        mock_any!(scan_vm_specific_roots(tls, Box::new(factory)))
    }
    fn notify_initial_thread_scan_complete(partial_scan: bool, tls: VMWorkerThread) {
        mock!(notify_initial_thread_scan_complete(partial_scan, tls))
    }
    fn supports_return_barrier() -> bool {
        mock!(supports_return_barrier())
    }
    fn prepare_for_roots_re_scanning() {
        mock!(prepare_for_roots_re_scanning())
    }
    fn process_weak_refs(
        worker: &mut GCWorker<Self>,
        tracer_context: impl ObjectTracerContext<Self>,
    ) -> bool {
        let worker: &'static mut GCWorker<Self> = lifetime!(worker);
        mock_any!(process_weak_refs(worker, tracer_context))
    }
    fn forward_weak_refs(
        worker: &mut GCWorker<Self>,
        tracer_context: impl ObjectTracerContext<Self>,
    ) {
        let worker: &'static mut GCWorker<Self> = lifetime!(worker);
        mock_any!(forward_weak_refs(worker, tracer_context))
    }
}
