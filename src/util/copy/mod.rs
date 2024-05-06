use std::mem::MaybeUninit;
use std::sync::Arc;

use crate::plan::PlanConstraints;
use crate::policy::copy_context::PolicyCopyContext;
use crate::policy::copyspace::CopySpace;
use crate::policy::copyspace::CopySpaceCopyContext;
use crate::policy::immix::ImmixSpace;
use crate::policy::immix::{ImmixCopyContext, ImmixHybridCopyContext};
use crate::policy::space::Space;
use crate::util::object_forwarding;
use crate::util::opaque_pointer::VMWorkerThread;
use crate::util::{Address, ObjectReference};
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use crate::MMTK;
use std::sync::atomic::Ordering;

use enum_map::Enum;
use enum_map::EnumMap;

use super::alloc::allocator::AllocatorContext;

const MAX_COPYSPACE_COPY_ALLOCATORS: usize = 1;
const MAX_IMMIX_COPY_ALLOCATORS: usize = 1;
const MAX_IMMIX_HYBRID_COPY_ALLOCATORS: usize = 1;

type CopySpaceMapping<VM> = Vec<(CopySelector, &'static dyn Space<VM>)>;

/// A configuration for GCWorkerCopyContext.
/// Similar to a `MutatorConfig`,
/// We expect each copying plan to provide a CopyConfig.
pub struct CopyConfig<VM: VMBinding> {
    /// Mapping CopySemantics to the actual copying allocators (CopySelector)
    pub(crate) copy_mapping: EnumMap<CopySemantics, CopySelector>,
    /// Mapping copying allocators with space
    pub(crate) space_mapping: CopySpaceMapping<VM>,
    /// A reference to the plan constraints.
    /// GCWorkerCopyContext may have plan-specific behaviors dependson the plan constraints.
    pub(crate) constraints: &'static PlanConstraints,
}

impl<VM: VMBinding> Default for CopyConfig<VM> {
    fn default() -> Self {
        CopyConfig {
            copy_mapping: EnumMap::default(),
            space_mapping: vec![],
            constraints: &crate::plan::DEFAULT_PLAN_CONSTRAINTS,
        }
    }
}

/// The thread local struct for each GC worker for copying. Each GC worker should include
/// one instance of this struct for copying operations.
pub struct GCWorkerCopyContext<VM: VMBinding> {
    /// Copy allocators for CopySpace
    pub copy: [MaybeUninit<CopySpaceCopyContext<VM>>; MAX_COPYSPACE_COPY_ALLOCATORS],
    /// Copy allocators for ImmixSpace
    pub immix: [MaybeUninit<ImmixCopyContext<VM>>; MAX_IMMIX_COPY_ALLOCATORS],
    /// Copy allocators for ImmixSpace
    pub immix_hybrid: [MaybeUninit<ImmixHybridCopyContext<VM>>; MAX_IMMIX_HYBRID_COPY_ALLOCATORS],
    /// The config for the plan
    config: CopyConfig<VM>,
}

impl<VM: VMBinding> GCWorkerCopyContext<VM> {
    /// Allocate for the object for GC copying.
    ///
    /// Arguments:
    /// * `original`: The original object that will be copied.
    /// * `bytes`: The size in bytes for the allocation.
    /// * `align`: The alignment in bytes for the allocation.
    /// * `offset`: The offset in bytes for the allocation.
    /// * `semantics`: The copy semantic for this coying allocation.
    ///   It determins which copy allocator will be used for the copying.
    pub fn alloc_copy(
        &mut self,
        original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: usize,
        semantics: CopySemantics,
    ) -> Address {
        #[cfg(debug_assertions)]
        if bytes > self.config.constraints.max_non_los_default_alloc_bytes {
            warn!(
                "Attempted to copy an object of {} bytes (> {}) which should be allocated with LOS and not be copied.",
                bytes, self.config.constraints.max_non_los_default_alloc_bytes
            );
        }
        match self.config.copy_mapping[semantics] {
            CopySelector::CopySpace(index) => {
                unsafe { self.copy[index as usize].assume_init_mut() }
                    .alloc_copy(original, bytes, align, offset)
            }
            CopySelector::Immix(index) => unsafe { self.immix[index as usize].assume_init_mut() }
                .alloc_copy(original, bytes, align, offset),
            CopySelector::ImmixHybrid(index) => {
                unsafe { self.immix_hybrid[index as usize].assume_init_mut() }
                    .alloc_copy(original, bytes, align, offset)
            }
            CopySelector::Unused => unreachable!(),
        }
    }

    /// Post allocation after allocating an object.
    ///
    /// Arguments:
    /// * `object`: The newly allocated object (the new object after copying).
    /// * `bytes`: The size of the object in bytes.
    /// * `semantics`: The copy semantic used for the copying.
    pub fn post_copy(&mut self, object: ObjectReference, bytes: usize, semantics: CopySemantics) {
        // Clear forwarding bits.
        object_forwarding::clear_forwarding_bits::<VM>(object);
        // If we are copying objects in mature space, we would need to mark the object as mature.
        if semantics.is_mature() && self.config.constraints.needs_log_bit {
            // If the plan uses unlogged bit, we set the unlogged bit (the object is unlogged/mature)
            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC
                .mark_byte_as_unlogged::<VM>(object, Ordering::Relaxed);
        }
        // Policy specific post copy.
        match self.config.copy_mapping[semantics] {
            CopySelector::CopySpace(index) => {
                unsafe { self.copy[index as usize].assume_init_mut() }.post_copy(object, bytes)
            }
            CopySelector::Immix(index) => {
                unsafe { self.immix[index as usize].assume_init_mut() }.post_copy(object, bytes)
            }
            CopySelector::ImmixHybrid(index) => {
                unsafe { self.immix_hybrid[index as usize].assume_init_mut() }
                    .post_copy(object, bytes)
            }
            CopySelector::Unused => unreachable!(),
        }
    }

    /// Prepare the copying allocators.
    pub fn prepare(&mut self) {
        // Delegate to prepare() for each policy copy context
        for (_, selector) in self.config.copy_mapping.iter() {
            match selector {
                CopySelector::CopySpace(index) => {
                    unsafe { self.copy[*index as usize].assume_init_mut() }.prepare()
                }
                CopySelector::Immix(index) => {
                    unsafe { self.immix[*index as usize].assume_init_mut() }.prepare()
                }
                CopySelector::ImmixHybrid(index) => {
                    unsafe { self.immix_hybrid[*index as usize].assume_init_mut() }.prepare()
                }
                CopySelector::Unused => {}
            }
        }
    }

    /// Release the copying allocators.
    pub fn release(&mut self) {
        // Delegate to release() for each policy copy context
        for (_, selector) in self.config.copy_mapping.iter() {
            match selector {
                CopySelector::CopySpace(index) => {
                    unsafe { self.copy[*index as usize].assume_init_mut() }.release()
                }
                CopySelector::Immix(index) => {
                    unsafe { self.immix[*index as usize].assume_init_mut() }.release()
                }
                CopySelector::ImmixHybrid(index) => {
                    unsafe { self.immix_hybrid[*index as usize].assume_init_mut() }.release()
                }
                CopySelector::Unused => {}
            }
        }
    }

    /// Create a GCWorkerCopyContext based on the configuration for a copying plan.
    ///
    /// Arguments:
    /// * `worker_tls`: The worker thread for this copy context.
    /// * `plan`: A reference to the current plan.
    /// * `config`: The configuration for the copy context.
    pub fn new(worker_tls: VMWorkerThread, mmtk: &MMTK<VM>, config: CopyConfig<VM>) -> Self {
        let mut ret = GCWorkerCopyContext {
            copy: unsafe { MaybeUninit::uninit().assume_init() },
            immix: unsafe { MaybeUninit::uninit().assume_init() },
            immix_hybrid: unsafe { MaybeUninit::uninit().assume_init() },
            config,
        };
        let context = Arc::new(AllocatorContext::new(mmtk));

        // Initiate the copy context for each policy based on the space mapping.
        for &(selector, space) in ret.config.space_mapping.iter() {
            match selector {
                CopySelector::CopySpace(index) => {
                    ret.copy[index as usize].write(CopySpaceCopyContext::new(
                        worker_tls,
                        context.clone(),
                        space.downcast_ref::<CopySpace<VM>>().unwrap(),
                    ));
                }
                CopySelector::Immix(index) => {
                    ret.immix[index as usize].write(ImmixCopyContext::new(
                        worker_tls,
                        context.clone(),
                        space.downcast_ref::<ImmixSpace<VM>>().unwrap(),
                    ));
                }
                CopySelector::ImmixHybrid(index) => {
                    ret.immix_hybrid[index as usize].write(ImmixHybridCopyContext::new(
                        worker_tls,
                        context.clone(),
                        space.downcast_ref::<ImmixSpace<VM>>().unwrap(),
                    ));
                }
                CopySelector::Unused => unreachable!(),
            }
        }

        ret
    }

    /// Create a stub GCWorkerCopyContext for non copying plans.
    pub fn new_non_copy() -> Self {
        GCWorkerCopyContext {
            copy: unsafe { MaybeUninit::uninit().assume_init() },
            immix: unsafe { MaybeUninit::uninit().assume_init() },
            immix_hybrid: unsafe { MaybeUninit::uninit().assume_init() },
            config: CopyConfig::default(),
        }
    }
}

/// CopySemantics describes the copying operation. It depends on
/// the kinds of GC, and the space. For example, in a mature/major GC in
/// a generational plan, the nursery should have `PromoteToMature` while
/// the mature space should have `Mature`.
/// This enum may be expanded in the future to describe more semantics.
#[derive(Clone, Copy, Enum, Debug)]
pub enum CopySemantics {
    /// The default copy behavior.
    DefaultCopy,
    /// Copy in nursery generation.
    Nursery,
    /// Promote an object from nursery to mature spaces.
    PromoteToMature,
    /// Copy in mature generation.
    Mature,
}

impl CopySemantics {
    /// Are we copying to a mature space?
    pub fn is_mature(&self) -> bool {
        matches!(self, CopySemantics::PromoteToMature | CopySemantics::Mature)
    }
}

#[repr(C, u8)]
#[derive(Copy, Clone, Debug, Default)]
pub(crate) enum CopySelector {
    CopySpace(u8),
    Immix(u8),
    ImmixHybrid(u8),
    #[default]
    Unused,
}
