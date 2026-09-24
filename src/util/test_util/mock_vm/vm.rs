// Some mock methods may get really complex
#![allow(clippy::type_complexity)]

use crate::plan::tracing::gc_work::root::DefaultRootsWorkFactory;
use crate::plan::tracing::gc_work::DefaultObjectTracerContext;
use crate::plan::tracing::UnsupportedTrace;
use crate::plan::ObjectQueue;
use crate::scheduler::*;
use crate::util::alloc::AllocationError;
use crate::util::copy::*;
use crate::util::heap::gc_trigger::GCTriggerPolicy;
use crate::util::opaque_pointer::*;
use crate::util::test_util;
use crate::util::test_util::mock_vm::thread_park::ThreadPark;
use crate::util::{Address, ObjectReference};
use crate::vm::object_model::specs::*;
use crate::vm::GCThreadContext;
use crate::vm::ObjectTracer;
use crate::vm::ObjectTracerContext;
use crate::vm::RootsWorkFactory;
use crate::vm::SlotVisitor;
use crate::vm::VMBinding;
use crate::Mutator;

use super::mock_method::*;
use crate::util::test_util::mock_vm::mock_api;

use std::any::Any;
use std::default::Default;
use std::ptr::NonNull;

/// The offset between object reference and the allocation address if we use
/// the default mock VM.
pub const DEFAULT_OBJECT_REF_OFFSET: usize = crate::util::constants::BYTES_IN_ADDRESS;

/// To mock VMBinding methods, we have to create a static instance of the mock VM.
/// A test may define its own mock VM type (see [`define_mock_vm`](crate::define_mock_vm)), so the
/// instance is type-erased, and is checked against the expected mock VM type when it is accessed.
/// There can only be one mock VM instance (and one MMTk instance) in a process.
static mut MOCK_VM_INSTANCE: Option<NonNull<dyn Any>> = None;

// MockVM only allows mock methods with references of no lifetime or static lifetime.
// If `VMBinding` methods has references of a specific lifetime,
// the references need to be turned into static lifetime before we can call mock methods.
// This is correct as long as we only use the references within the mock methods, and
// we do not store them for access after the mock method returns.
macro_rules! lifetime {
    ($e: expr) => {
        unsafe {
            // The dynamic nature of this lifetime-removal macro makes it impossible to reason
            // about the source and destination types of the `transmute`.
            #[allow(clippy::missing_transmute_annotations)]
            std::mem::transmute($e)
        }
    };
}

/// Call `MockMethod`.
macro_rules! mock {
    ($fn: ident($($arg:expr),*)) => {
        {
            let arg_tuple = ($($arg),*);
            Self::write_mockvm(|mock| mock.$fn.call(arg_tuple))
        }
    };
}
/// Call `MockAny`.
#[allow(unused_macros)] // This macro is unused for now.
macro_rules! mock_any {
    ($fn: ident($($arg:expr),*)) => {
        {
            let arg_tuple = ($($arg),*);
            *Self::write_mockvm(|mock| mock.$fn.call_any(Box::new(arg_tuple))).downcast().unwrap()
        }
    };
}

/// Initialize the static MockVM instance.
pub fn init_mockvm(mockvm: MockVM) {
    MockVM::init_mockvm(mockvm)
}

/// Read from the static MockVM instance.
pub fn read_mockvm<F, R>(func: F) -> R
where
    F: FnOnce(&MockVM) -> R,
{
    MockVM::read_mockvm(func)
}
/// Write to the static MockVM instance.
pub fn write_mockvm<F, R>(func: F) -> R
where
    F: FnOnce(&mut MockVM) -> R,
{
    MockVM::write_mockvm(func)
}

/// A test that uses `MockVM` should use this method to wrap the entire test
/// that may use `MockVM`. A test that uses a custom mock VM type should use
/// [`GenericMockVM::with_mockvm`] instead, e.g. `CustomVM::with_mockvm(...)`.
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
    MockVM::with_mockvm(setup, test, cleanup)
}

/// Set up a default `MockVM`
pub fn default_setup() -> MockVM {
    MockVM::default()
}

/// No extra clean up after the test.
pub fn no_cleanup() {}

/// The configuration of a mock VM type. It provides all the constants and the associated types
/// that the VM traits (`VMBinding`, `ObjectModel`, `ReferenceGlue`, `Scanning`, etc) would require,
/// so a test can create a mock VM type ([`GenericMockVM<C>`]) with the configuration it needs.
///
/// Use [`define_mock_vm`](crate::define_mock_vm) to define a configuration and the mock VM type.
/// The macro provides the default associated types (Rust does not support defaults for associated
/// types yet). The constants have default values here, which are the same as the defaults in the
/// VM traits, unless noted otherwise.
pub trait MockVMConfig: 'static + Send + Sync {
    // VMBinding

    /// See [`VMBinding::VMSlot`]
    type VMSlot: crate::vm::slot::Slot;
    /// See [`VMBinding::VMMemorySlice`]
    type VMMemorySlice: crate::vm::slot::MemorySlice<SlotType = Self::VMSlot>;
    /// See [`VMBinding::ALIGNMENT_VALUE`]
    const ALIGNMENT_VALUE: u8 = crate::vm::DEFAULT_ALIGNMENT_VALUE;
    /// See [`VMBinding::MIN_ALIGNMENT`]
    const MIN_ALIGNMENT: usize = 1 << crate::vm::DEFAULT_LOG_MIN_ALIGNMENT;
    /// See [`VMBinding::MAX_ALIGNMENT`]. MockVM uses 64 bytes by default, unlike `VMBinding`.
    const MAX_ALIGNMENT: usize = 1 << 6;
    /// See [`VMBinding::USE_ALLOCATION_OFFSET`]
    const USE_ALLOCATION_OFFSET: bool = crate::vm::DEFAULT_USE_ALLOCATION_OFFSET;
    /// See [`VMBinding::ALLOC_END_ALIGNMENT`]
    const ALLOC_END_ALIGNMENT: usize = crate::vm::DEFAULT_ALLOC_END_ALIGNMENT;

    // ObjectModel. MockVM places most of the metadata in the header by default.

    /// See [`crate::vm::ObjectModel::GLOBAL_LOG_BIT_SPEC`]
    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec = VMGlobalLogBitSpec::in_header(0);
    /// See [`crate::vm::ObjectModel::GLOBAL_FIELD_UNLOG_BIT_SPEC`]
    const GLOBAL_FIELD_UNLOG_BIT_SPEC: VMGlobalFieldUnlogBitSpec =
        VMGlobalFieldUnlogBitSpec::side_first();
    /// See [`crate::vm::ObjectModel::LOCAL_FORWARDING_POINTER_SPEC`]
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec =
        VMLocalForwardingPointerSpec::in_header(0);
    /// See [`crate::vm::ObjectModel::LOCAL_FORWARDING_BITS_SPEC`]
    const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec =
        VMLocalForwardingBitsSpec::in_header(0);
    /// See [`crate::vm::ObjectModel::LOCAL_MARK_BIT_SPEC`]. LXR clears mark bits in bulk over
    /// side metadata, so this is a side spec by default (unlike most of the other local specs
    /// here, which stay in the header).
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec = VMLocalMarkBitSpec::side_first();
    /// See [`crate::vm::ObjectModel::LOCAL_PINNING_BIT_SPEC`]
    #[cfg(feature = "object_pinning")]
    const LOCAL_PINNING_BIT_SPEC: VMLocalPinningBitSpec = VMLocalPinningBitSpec::in_header(0);
    /// See [`crate::vm::ObjectModel::LOCAL_LOS_MARK_NURSERY_SPEC`]
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec =
        VMLocalLOSMarkNurserySpec::in_header(0);
    /// See [`crate::vm::ObjectModel::COMPRESSED_PTR_ENABLED`]
    const COMPRESSED_PTR_ENABLED: bool = crate::vm::object_model::DEFAULT_COMPRESSED_PTR_ENABLED;
    /// See [`crate::vm::ObjectModel::NEED_VO_BITS_DURING_TRACING`]
    #[cfg(feature = "vo_bit")]
    const NEED_VO_BITS_DURING_TRACING: bool =
        crate::vm::object_model::DEFAULT_NEED_VO_BITS_DURING_TRACING;
    /// See [`crate::vm::ObjectModel::VM_WORST_CASE_COPY_EXPANSION`]
    const VM_WORST_CASE_COPY_EXPANSION: f64 =
        crate::vm::object_model::DEFAULT_VM_WORST_CASE_COPY_EXPANSION;
    /// See [`crate::vm::ObjectModel::UNIFIED_OBJECT_REFERENCE_ADDRESS`]
    const UNIFIED_OBJECT_REFERENCE_ADDRESS: bool =
        crate::vm::object_model::DEFAULT_UNIFIED_OBJECT_REFERENCE_ADDRESS;
    /// See [`crate::vm::ObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND`]. MockVM uses
    /// [`DEFAULT_OBJECT_REF_OFFSET`] by default.
    const OBJECT_REF_OFFSET_LOWER_BOUND: isize = DEFAULT_OBJECT_REF_OFFSET as isize;

    // ReferenceGlue

    /// See [`crate::vm::ReferenceGlue::FinalizableType`]
    type FinalizableType: crate::vm::Finalizable;

    // Scanning

    /// See [`crate::vm::Scanning::UNIQUE_OBJECT_ENQUEUING`]
    const UNIQUE_OBJECT_ENQUEUING: bool = crate::vm::DEFAULT_UNIQUE_OBJECT_ENQUEUING;
}

/// Define a mock VM type with a custom [`MockVMConfig`]. This defines the config type, and a type
/// alias for [`GenericMockVM`] with the config. The body is the items of the `MockVMConfig`
/// implementation: it may override any constants, and any of the associated types (which default
/// to `VMSlot = Address`, `VMMemorySlice = Range<Address>` and `FinalizableType = ObjectReference`).
///
/// ```ignore
/// define_mock_vm! {
///     /// A mock VM with forwarding bits and mark bits on the side.
///     type CustomVM = GenericMockVM<CustomConfig> {
///         const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec =
///             VMLocalForwardingBitsSpec::side_first();
///         const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec =
///             VMLocalMarkBitSpec::side_after(Self::LOCAL_FORWARDING_BITS_SPEC.as_spec());
///     }
/// }
///
/// #[test]
/// fn test() {
///     CustomVM::with_mockvm(
///         CustomVM::default,
///         || {
///             let fixture = GenericMutatorFixture::<CustomVM>::create();
///             // ...
///         },
///         no_cleanup,
///     )
/// }
/// ```
///
/// Note that when a constant is overridden, the other constants that depend on it are not
/// changed. For example, when a local metadata spec is moved to the side, make sure it does not
/// overlap with the other local side metadata specs (use `side_after()`).
///
/// Each mock VM type is a different `VMBinding`, so MMTk is compiled again for it. Only define a
/// mock VM type when the test needs a different configuration.
#[macro_export]
macro_rules! define_mock_vm {
    // Pick the given type, or the default type.
    (@or [] $default:ty) => { $default };
    (@or [$t:ty] $default:ty) => { $t };

    // Munch the items in the body. The state is
    // [header] [VMSlot] [VMMemorySlice] [FinalizableType] [consts].
    (@munch $head:tt [] $slice:tt $fin:tt $consts:tt type VMSlot = $t:ty; $($rest:tt)*) => {
        $crate::define_mock_vm!(@munch $head [$t] $slice $fin $consts $($rest)*);
    };
    (@munch $head:tt $slot:tt [] $fin:tt $consts:tt type VMMemorySlice = $t:ty; $($rest:tt)*) => {
        $crate::define_mock_vm!(@munch $head $slot [$t] $fin $consts $($rest)*);
    };
    (@munch $head:tt $slot:tt $slice:tt [] $consts:tt type FinalizableType = $t:ty; $($rest:tt)*) => {
        $crate::define_mock_vm!(@munch $head $slot $slice [$t] $consts $($rest)*);
    };
    (@munch $head:tt $slot:tt $slice:tt $fin:tt [$($consts:tt)*]
        $(#[$meta:meta])* const $cname:ident : $cty:ty = $cval:expr; $($rest:tt)*) => {
        $crate::define_mock_vm!(@munch $head $slot $slice $fin
            [$($consts)* $(#[$meta])* const $cname: $cty = $cval;] $($rest)*);
    };
    // Done. Emit the config and the type alias.
    (@munch [$(#[$attr:meta])* $vis:vis $name:ident $config:ident]
        [$($slot:ty)?] [$($slice:ty)?] [$($fin:ty)?] [$($consts:tt)*]) => {
        #[doc = concat!("The [`MockVMConfig`](", "crate::util::test_util::mock_vm::MockVMConfig) for [`", stringify!($name), "`].")]
        #[derive(Default)]
        $vis struct $config;

        impl $crate::util::test_util::mock_vm::MockVMConfig for $config {
            type VMSlot = $crate::define_mock_vm!(@or [$($slot)?] $crate::util::Address);
            type VMMemorySlice =
                $crate::define_mock_vm!(@or [$($slice)?] ::std::ops::Range<$crate::util::Address>);
            type FinalizableType =
                $crate::define_mock_vm!(@or [$($fin)?] $crate::util::ObjectReference);
            $($consts)*
        }

        $(#[$attr])*
        $vis type $name = $crate::util::test_util::mock_vm::GenericMockVM<$config>;
    };

    // Entry
    (
        $(#[$attr:meta])*
        $vis:vis type $name:ident = GenericMockVM<$config:ident> { $($body:tt)* }
    ) => {
        $crate::define_mock_vm!(@munch [$(#[$attr])* $vis $name $config] [] [] [] [] $($body)*);
    };
}

define_mock_vm! {
    /// The default mock VM type. Most tests should use this type.
    pub type MockVM = GenericMockVM<DefaultMockVMConfig> {}
}

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
/// `Box::new((MockMethod::<(&'static mut GCWorker<Self>,DefaultObjectTracerContext<UnsupportedTrace<Self>>,),bool>::new_unimplemented())`.
///
/// Note that when `MockAny` is used, one needs to make sure that the types of the actual arguments match the argument types used for creating the `MockMethod`.
/// We provide a default implementation for those `MockAny` methods, and it is very possible that the types in the default implementation do not
/// match the arguments you would like to test with. You should overwrite the default `MockMethod` during the MockVM setup.
///
/// # Mock constants and associated types
///
/// Constants and associated types are part of the type, so they cannot be changed in a mock VM
/// instance. Instead, they are provided by the config type `C` ([`MockVMConfig`]). [`MockVM`] is
/// the mock VM type with the default config. A test that needs different constants or associated
/// types can define its own mock VM type with [`define_mock_vm`](crate::define_mock_vm).
// The current implementation is not perfect, but at least it works, and it is easy enough to debug with.
// I have tried different third-party libraries for mocking, and each has its own limitation. And
// none of the libraries I tried can mock `VMBinding` and the associated traits out of box. Even after I attempted
// to remove all those VM traits and had all the methods in `VMBinding`, the libraries still did not
// work out.
pub struct GenericMockVM<C: MockVMConfig> {
    // active plan
    pub number_of_mutators: MockMethod<(), usize>,
    pub is_mutator: MockMethod<VMThread, bool>,
    pub mutator: MockMethod<VMMutatorThread, &'static mut Mutator<Self>>,
    pub mutators: MockMethod<(), Box<dyn Iterator<Item = &'static mut Mutator<Self>> + 'static>>,
    pub vm_trace_object: MockMethod<
        (
            &'static mut dyn ObjectQueue,
            ObjectReference,
            &'static mut GCWorker<Self>,
        ),
        ObjectReference,
    >,
    // collection
    pub stop_all_mutators:
        MockMethod<(VMWorkerThread, Box<dyn FnMut(&'static mut Mutator<Self>)>), ()>,
    pub resume_mutators: MockMethod<VMWorkerThread, ()>,
    pub block_for_gc: MockMethod<VMMutatorThread, ()>,
    pub spawn_gc_thread: MockMethod<(VMThread, GCThreadContext<Self>), ()>,
    pub out_of_memory: MockMethod<(VMThread, AllocationError), ()>,
    pub schedule_finalization: MockMethod<VMWorkerThread, ()>,
    pub post_forwarding: MockMethod<VMWorkerThread, ()>,
    pub vm_live_bytes: MockMethod<(), usize>,
    pub create_gc_trigger: MockMethod<(), Box<dyn GCTriggerPolicy<Self>>>,
    // object model
    pub copy_object: MockMethod<
        (
            ObjectReference,
            CopySemantics,
            &'static GCWorkerCopyContext<Self>,
        ),
        ObjectReference,
    >,
    pub try_copy_object: MockMethod<
        (
            ObjectReference,
            CopySemantics,
            &'static GCWorkerCopyContext<Self>,
        ),
        Option<ObjectReference>,
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
    pub dump_object: MockMethod<ObjectReference, ()>,
    // reference glue
    pub weakref_clear_referent: MockMethod<ObjectReference, ()>,
    pub weakref_set_referent: MockMethod<(ObjectReference, ObjectReference), ()>,
    pub weakref_get_referent: MockMethod<ObjectReference, Option<ObjectReference>>,
    pub weakref_enqueue_references: MockMethod<(&'static [ObjectReference], VMWorkerThread), ()>,
    // scanning
    pub support_slot_enqueuing: MockMethod<(VMWorkerThread, ObjectReference), bool>,
    pub scan_object: MockMethod<
        (
            VMWorkerThread,
            ObjectReference,
            &'static mut dyn SlotVisitor<C::VMSlot>,
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

/// This struct is used to hold a pointer to a `Mutator<VM>` for a mock VM type.
/// The pointer to this struct is used as the 'mutator tls' pointer for MMTK.
/// The mutator is type-erased so we can check its type when we access it.
#[derive(Clone)]
pub struct MutatorHandle {
    pub ptr: *mut dyn Any,
}

impl MutatorHandle {
    /// Bind a mutator for the mock VM type `VM`, and return its mutator tls.
    pub fn bind<VM: VMBinding>() -> VMMutatorThread {
        let mmtk = mock_api::singleton::<VM>();

        let mutator_handle = Box::new(MutatorHandle {
            ptr: std::ptr::null_mut::<Mutator<VM>>(),
        });
        let mutator_handle_ptr = Box::into_raw(mutator_handle);
        let tls = VMMutatorThread(VMThread(OpaquePointer::from_address(
            Address::from_mut_ptr(mutator_handle_ptr),
        )));

        let mutator = crate::memory_manager::bind_mutator(mmtk, tls);
        let mutator_ptr: *mut Mutator<VM> = Box::into_raw(mutator);

        unsafe {
            (*mutator_handle_ptr).ptr = mutator_ptr;
        }

        MUTATOR_PARK.register(tls.0);
        tls
    }

    pub fn as_mutator<VM: VMBinding>(&self) -> &'static mut Mutator<VM> {
        assert!(!self.ptr.is_null(), "The mutator is not bound yet");
        unsafe { &mut *self.ptr }
            .downcast_mut::<Mutator<VM>>()
            .unwrap_or_else(|| {
                panic!(
                    "The mutator is not a {}",
                    std::any::type_name::<Mutator<VM>>()
                )
            })
    }
}

unsafe impl Sync for MutatorHandle {}
unsafe impl Send for MutatorHandle {}

impl VMMutatorThread {
    /// Get a mutable reference to the underlying Mutator<MockVM>.
    pub fn as_mock_mutator(self) -> &'static mut Mutator<MockVM> {
        self.as_generic_mock_mutator()
    }

    /// Get a mutable reference to the underlying mutator for a mock VM type.
    pub fn as_generic_mock_mutator<VM: VMBinding>(self) -> &'static mut Mutator<VM> {
        unsafe { &*self.0 .0.to_address().to_ptr::<MutatorHandle>() }.as_mutator()
    }
}

lazy_static! {
    pub static ref MUTATOR_PARK: ThreadPark = ThreadPark::new("mutators");
    // We never really park GC threads. We just reuse this struct to track GC threads.
    pub static ref GC_THREADS: ThreadPark = ThreadPark::new("gc workers");
}

fn current_thread_tls() -> VMThread {
    VMThread(OpaquePointer::from_address(unsafe {
        Address::from_usize(thread_id::get())
    }))
}

impl<C: MockVMConfig> Default for GenericMockVM<C> {
    fn default() -> Self {
        Self {
            number_of_mutators: MockMethod::new_fixed(Box::new(|()| {
                // Just return the number of registered mutator threads
                MUTATOR_PARK.number_of_threads()
            })),
            is_mutator: MockMethod::new_fixed(Box::new(|tls: VMThread| {
                MUTATOR_PARK.is_thread(tls)
            })),
            mutator: MockMethod::new_fixed(Box::new(|tls| tls.as_generic_mock_mutator())),
            mutators: MockMethod::new_fixed(Box::new(|()| {
                // Just return an iterator over all registered mutators
                let mutators: Vec<&'static mut Mutator<Self>> = MUTATOR_PARK
                    .all_threads()
                    .into_iter()
                    .map(|tls| VMMutatorThread(tls).as_generic_mock_mutator())
                    .collect();
                Box::new(mutators.into_iter())
            })),
            vm_trace_object: MockMethod::new_fixed(Box::new(|(_, object, _)| {
                panic!("MMTk cannot trace object {:?} as it does not belong to any MMTk space. If the object is known to the VM, the binding can override this method and handle its tracing.", object)
            })),

            stop_all_mutators: MockMethod::new_fixed(Box::new(|(_tls, mut mutator_visitor)| {
                info!("Waiting for all threads to park...");
                MUTATOR_PARK.wait_all_parked();
                info!("All threads are parked.");

                MUTATOR_PARK.all_threads().into_iter().for_each(|tls| {
                    mutator_visitor(VMMutatorThread(tls).as_generic_mock_mutator())
                });
            })),
            resume_mutators: MockMethod::new_fixed(Box::new(|_tls| {
                info!("Resuming all parked threads...");
                MUTATOR_PARK.unpark_all();
            })),
            block_for_gc: MockMethod::new_fixed(Box::new(|tls| {
                MUTATOR_PARK.park(tls.0);
            })),
            spawn_gc_thread: MockMethod::new_fixed(Box::new(|(_parent_tls, ctx)| {
                // Just drop the join handle. The thread will run until the process quits.
                let _ = std::thread::Builder::new()
                    .name("MMTk Worker".to_string())
                    .spawn(move || {
                        // Start the worker loop
                        let worker_tls = VMWorkerThread(current_thread_tls());
                        GC_THREADS.register(worker_tls.0);
                        match ctx {
                            GCThreadContext::Worker(w) => crate::memory_manager::start_worker(
                                mock_api::singleton::<Self>(),
                                worker_tls,
                                w,
                            ),
                        }
                        GC_THREADS.unregister(worker_tls.0);
                    });
            })),
            out_of_memory: MockMethod::new_fixed(Box::new(|(_, err)| {
                panic!("Out of memory with {:?}!", err)
            })),
            schedule_finalization: MockMethod::new_default(),
            post_forwarding: MockMethod::new_default(),
            vm_live_bytes: MockMethod::new_default(),
            create_gc_trigger: MockMethod::new_unimplemented(),

            copy_object: MockMethod::new_unimplemented(),
            try_copy_object: MockMethod::new_unimplemented(),
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
            dump_object: MockMethod::new_unimplemented(),

            weakref_clear_referent: MockMethod::new_unimplemented(),
            weakref_get_referent: MockMethod::new_unimplemented(),
            weakref_set_referent: MockMethod::new_unimplemented(),
            weakref_enqueue_references: MockMethod::new_unimplemented(),

            support_slot_enqueuing: MockMethod::new_fixed(Box::new(|_| true)),
            scan_object: MockMethod::new_unimplemented(),
            scan_object_and_trace_edges: MockMethod::new_unimplemented(),
            // We instantiate a `MockMethod` with the arguments as `DefaultRootsWorkFactory<..., UnsupportedTrace<MockVM>, ...>`,
            // thus the mock method expects the actual call arguments to match the type.
            // In most cases, this won't work and this `MockMethod` is just a place holder. It is
            // fine as long as the method is not actually called.
            // If the user will need this method, and would like to mock the method in their particular test,
            // they are expected to provide their own
            // `MockMethod` that matches the argument types they will pass for the test case.
            // See the documents on the section about `MockAny` on the `GenericMockVM` type.
            scan_roots_in_mutator_thread: Box::new(MockMethod::<
                (
                    VMWorkerThread,
                    &'static mut Mutator<Self>,
                    DefaultRootsWorkFactory<Self, UnsupportedTrace<Self>, UnsupportedTrace<Self>>,
                ),
                (),
            >::new_unimplemented()),
            // Same here: the `MockMethod` is just a place holder. See the above comments.
            scan_vm_specific_roots: Box::new(MockMethod::<
                (
                    VMWorkerThread,
                    DefaultRootsWorkFactory<Self, UnsupportedTrace<Self>, UnsupportedTrace<Self>>,
                ),
                (),
            >::new_unimplemented()),
            notify_initial_thread_scan_complete: MockMethod::new_fixed(Box::new(|(_, _)| {})),
            supports_return_barrier: MockMethod::new_unimplemented(),
            prepare_for_roots_re_scanning: MockMethod::new_fixed(Box::new(|_| {
                warn!("prepare_for_roots_re_scanning called on MockVM, it is empty at the moment.");
            })),
            // Same here: the `MockMethod` is just a place holder. See the above comments.
            process_weak_refs: Box::new(MockMethod::<
                (
                    &'static mut GCWorker<Self>,
                    DefaultObjectTracerContext<UnsupportedTrace<Self>>,
                ),
                bool,
            >::new_unimplemented()),
            // Same here: the `MockMethod` is just a place holder. See the above comments.
            forward_weak_refs: Box::new(MockMethod::<
                (
                    &'static mut GCWorker<Self>,
                    DefaultObjectTracerContext<UnsupportedTrace<Self>>,
                ),
                (),
            >::new_default()),
        }
    }
}

unsafe impl<C: MockVMConfig> Sync for GenericMockVM<C> {}
unsafe impl<C: MockVMConfig> Send for GenericMockVM<C> {}

impl<C: MockVMConfig> VMBinding for GenericMockVM<C> {
    type VMSlot = C::VMSlot;
    type VMMemorySlice = C::VMMemorySlice;

    type VMActivePlan = Self;
    type VMCollection = Self;
    type VMObjectModel = Self;
    type VMReferenceGlue = Self;
    type VMScanning = Self;

    const ALIGNMENT_VALUE: u8 = C::ALIGNMENT_VALUE;
    const MIN_ALIGNMENT: usize = C::MIN_ALIGNMENT;
    const MAX_ALIGNMENT: usize = C::MAX_ALIGNMENT;
    const USE_ALLOCATION_OFFSET: bool = C::USE_ALLOCATION_OFFSET;
    const ALLOC_END_ALIGNMENT: usize = C::ALLOC_END_ALIGNMENT;
}

impl<C: MockVMConfig> crate::vm::ActivePlan<GenericMockVM<C>> for GenericMockVM<C> {
    fn number_of_mutators() -> usize {
        mock!(number_of_mutators())
    }

    fn is_mutator(tls: VMThread) -> bool {
        mock!(is_mutator(tls))
    }

    fn mutator(tls: VMMutatorThread) -> &'static mut Mutator<Self> {
        mock!(mutator(tls))
    }

    fn mutators<'a>() -> Box<dyn Iterator<Item = &'a mut Mutator<Self>> + 'a> {
        let ret = mock!(mutators());
        lifetime!(ret)
    }

    fn vm_trace_object<Q: ObjectQueue>(
        queue: &mut Q,
        object: ObjectReference,
        worker: &mut GCWorker<Self>,
    ) -> ObjectReference {
        mock!(vm_trace_object(
            lifetime!(queue as &mut dyn ObjectQueue),
            object,
            lifetime!(worker)
        ))
    }
}

impl<C: MockVMConfig> crate::vm::Collection<GenericMockVM<C>> for GenericMockVM<C> {
    fn stop_all_mutators<F>(tls: VMWorkerThread, mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<Self>),
    {
        mock!(stop_all_mutators(
            tls,
            lifetime!(Box::new(mutator_visitor) as Box<dyn FnMut(&'static mut Mutator<Self>)>)
        ))
    }

    fn resume_mutators(tls: VMWorkerThread) {
        mock!(resume_mutators(tls))
    }

    fn block_for_gc(tls: VMMutatorThread) {
        mock!(block_for_gc(tls))
    }

    fn spawn_gc_thread(tls: VMThread, ctx: GCThreadContext<Self>) {
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

    fn vm_live_bytes() -> usize {
        mock!(vm_live_bytes())
    }

    fn create_gc_trigger() -> Box<dyn GCTriggerPolicy<Self>> {
        mock!(create_gc_trigger())
    }
}

impl<C: MockVMConfig> crate::vm::ObjectModel<GenericMockVM<C>> for GenericMockVM<C> {
    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec = C::GLOBAL_LOG_BIT_SPEC;
    const GLOBAL_FIELD_UNLOG_BIT_SPEC: VMGlobalFieldUnlogBitSpec = C::GLOBAL_FIELD_UNLOG_BIT_SPEC;
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec =
        C::LOCAL_FORWARDING_POINTER_SPEC;
    const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec = C::LOCAL_FORWARDING_BITS_SPEC;
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec = C::LOCAL_MARK_BIT_SPEC;
    #[cfg(feature = "object_pinning")]
    const LOCAL_PINNING_BIT_SPEC: VMLocalPinningBitSpec = C::LOCAL_PINNING_BIT_SPEC;
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec = C::LOCAL_LOS_MARK_NURSERY_SPEC;
    const COMPRESSED_PTR_ENABLED: bool = C::COMPRESSED_PTR_ENABLED;
    #[cfg(feature = "vo_bit")]
    const NEED_VO_BITS_DURING_TRACING: bool = C::NEED_VO_BITS_DURING_TRACING;
    const VM_WORST_CASE_COPY_EXPANSION: f64 = C::VM_WORST_CASE_COPY_EXPANSION;
    const UNIFIED_OBJECT_REFERENCE_ADDRESS: bool = C::UNIFIED_OBJECT_REFERENCE_ADDRESS;
    const OBJECT_REF_OFFSET_LOWER_BOUND: isize = C::OBJECT_REF_OFFSET_LOWER_BOUND;

    fn copy(
        from: ObjectReference,
        semantics: CopySemantics,
        copy_context: &mut GCWorkerCopyContext<Self>,
    ) -> ObjectReference {
        mock!(copy_object(from, semantics, lifetime!(copy_context)))
    }

    fn try_copy(
        from: ObjectReference,
        semantics: CopySemantics,
        copy_context: &mut GCWorkerCopyContext<Self>,
    ) -> Option<ObjectReference> {
        mock!(try_copy_object(from, semantics, lifetime!(copy_context)))
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

    fn dump_object(object: ObjectReference) {
        mock!(dump_object(object))
    }
}

impl<C: MockVMConfig> crate::vm::ReferenceGlue<GenericMockVM<C>> for GenericMockVM<C> {
    type FinalizableType = C::FinalizableType;

    fn clear_referent(new_reference: ObjectReference) {
        mock!(weakref_clear_referent(new_reference))
    }

    fn set_referent(reference: ObjectReference, referent: ObjectReference) {
        mock!(weakref_set_referent(reference, referent))
    }
    fn get_referent(object: ObjectReference) -> Option<ObjectReference> {
        mock!(weakref_get_referent(object))
    }
    fn enqueue_references(references: &[ObjectReference], tls: VMWorkerThread) {
        mock!(weakref_enqueue_references(lifetime!(references), tls))
    }
}

impl<C: MockVMConfig> crate::vm::Scanning<GenericMockVM<C>> for GenericMockVM<C> {
    const UNIQUE_OBJECT_ENQUEUING: bool = C::UNIQUE_OBJECT_ENQUEUING;

    fn support_slot_enqueuing(tls: VMWorkerThread, object: ObjectReference) -> bool {
        mock!(support_slot_enqueuing(tls, object))
    }
    fn scan_object(
        tls: VMWorkerThread,
        object: ObjectReference,
        slot_visitor: &mut impl SlotVisitor<C::VMSlot>,
    ) {
        mock!(scan_object(
            tls,
            object,
            lifetime!(slot_visitor as &mut dyn SlotVisitor<C::VMSlot>)
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
        _tls: VMWorkerThread,
        _mutator: &'static mut Mutator<Self>,
        _factory: impl RootsWorkFactory<C::VMSlot>,
    ) {
        // mock_any!(scan_roots_in_mutator_thread(
        //     tls,
        //     mutator,
        //     Box::new(factory)
        // ))
        warn!("scan_roots_in_mutator_thread is not properly mocked. The default implementation does nothing.");
    }
    fn scan_vm_specific_roots(_tls: VMWorkerThread, _factory: impl RootsWorkFactory<C::VMSlot>) {
        // mock_any!(scan_vm_specific_roots(tls, Box::new(factory)))
        warn!("scan_vm_specific_roots is not properly mocked. The default implementation does nothing.");
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
        _worker: &mut GCWorker<Self>,
        _tracer_context: impl ObjectTracerContext<Self>,
    ) -> bool {
        // let worker: &'static mut GCWorker<Self> = lifetime!(worker);
        // mock_any!(process_weak_refs(worker, tracer_context))
        warn!("process_weak_refs is not properly mocked. The default implementation does nothing.");
        false
    }
    fn forward_weak_refs(
        _worker: &mut GCWorker<Self>,
        _tracer_context: impl ObjectTracerContext<Self>,
    ) {
        // let worker: &'static mut GCWorker<Self> = lifetime!(worker);
        // mock_any!(forward_weak_refs(worker, tracer_context))
        warn!("forward_weak_refs is not properly mocked. The default implementation does nothing.");
    }
}

impl<C: MockVMConfig> GenericMockVM<C> {
    /// Initialize the static mock VM instance.
    pub fn init_mockvm(mockvm: Self) {
        unsafe {
            if (*std::ptr::addr_of!(MOCK_VM_INSTANCE)).is_some() {
                warn!("MockVM is already initialized. Overwriting the existing instance. This may change the behavior of MockVM.");
            }
            let boxed: Box<dyn Any> = Box::new(mockvm);
            MOCK_VM_INSTANCE = Some(NonNull::new_unchecked(Box::into_raw(boxed)));
        }
    }

    fn instance() -> &'static mut Self {
        let ptr = unsafe { MOCK_VM_INSTANCE }
            .expect("MockVM is not initialized. Use with_mockvm() to run the test.");
        unsafe { &mut *ptr.as_ptr() }
            .downcast_mut::<Self>()
            .unwrap_or_else(|| {
                panic!(
                    "The initialized mock VM is not a {}",
                    std::any::type_name::<Self>()
                )
            })
    }

    /// Read from the static mock VM instance.
    pub fn read_mockvm<F, R>(func: F) -> R
    where
        F: FnOnce(&Self) -> R,
    {
        func(Self::instance())
    }

    /// Write to the static mock VM instance.
    pub fn write_mockvm<F, R>(func: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        func(Self::instance())
    }

    /// A test that uses this mock VM type should use this method to wrap the entire test.
    /// See [`with_mockvm`](crate::util::test_util::mock_vm::with_mockvm).
    pub fn with_mockvm<S, T, CL>(setup: S, test: T, cleanup: CL)
    where
        S: FnOnce() -> Self,
        T: FnOnce() + std::panic::UnwindSafe,
        CL: FnOnce(),
    {
        test_util::serial_test(|| {
            {
                use std::panic;
                let orig_hook = panic::take_hook();
                panic::set_hook(Box::new(move |panic_info| {
                    let current_tls = current_thread_tls();
                    if GC_THREADS.is_thread(current_tls) {
                        use std::backtrace::Backtrace;
                        let bt = Backtrace::force_capture();

                        // If this is a GC thread, we make the whole process abort.
                        error!(
                            "Panic occurred in GC thread with MockVM. Aborting the process. \n{}",
                            panic_info
                        );
                        error!("Backtrace:\n{}", bt);
                        std::process::exit(1);
                    } else {
                        // invoke the default handler
                        orig_hook(panic_info);
                    }
                }));
            }
            // Setup
            {
                Self::init_mockvm(setup());
            }
            test_util::with_cleanup(test, cleanup);
        })
    }

    /// Bind a mutator to the MMTk singleton for this mock VM type.
    pub fn bind_mutator() -> VMMutatorThread {
        MutatorHandle::bind::<Self>()
    }

    pub fn object_start_to_ref(start: Address) -> ObjectReference {
        ObjectReference::from_raw_address(start + DEFAULT_OBJECT_REF_OFFSET).unwrap()
    }
}
