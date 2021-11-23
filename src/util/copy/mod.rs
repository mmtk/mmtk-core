use std::mem::MaybeUninit;

use crate::vm::VMBinding;
use crate::policy::copy_context::PolicyCopyContext;
use crate::policy::copyspace::CopySpaceCopyContext;
use crate::policy::immix::ImmixCopyContext;
use crate::util::{Address, ObjectReference};
use crate::plan::PlanConstraints;
use crate::util::opaque_pointer::VMWorkerThread;
use crate::policy::space::Space;
use crate::scheduler::GCWorkerLocal;
use crate::plan::Plan;
use crate::policy::copyspace::CopySpace;
use crate::policy::immix::ImmixSpace;
use std::sync::atomic::Ordering;
use crate::util::object_forwarding;
use crate::vm::ObjectModel;

use enum_map::Enum;
use enum_map::EnumMap;

const MAX_COPYSPACE_COPY_ALLOCATORS: usize = 1;
const MAX_IMMIX_COPY_ALLOCATORS: usize = 2;

pub struct CopyConfig {
    pub copy_mapping: EnumMap<CopySemantics, CopySelector>,
    pub constraints: &'static PlanConstraints,
}

pub struct GCWorkerCopyContext<VM: VMBinding> {
    pub copy: [MaybeUninit<CopySpaceCopyContext<VM>>; MAX_COPYSPACE_COPY_ALLOCATORS],
    pub immix: [MaybeUninit<ImmixCopyContext<VM>>; MAX_IMMIX_COPY_ALLOCATORS],
    config: CopyConfig,
}

impl<VM: VMBinding> GCWorkerCopyContext<VM> {
    pub fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, semantics: CopySemantics) -> Address {
        match self.config.copy_mapping[semantics] {
            CopySelector::CopySpace(index) => unsafe { self.copy[index as usize].assume_init_mut() }.alloc_copy(original, bytes, align, offset, semantics),
            CopySelector::Immix(index) => unsafe { self.immix[index as usize].assume_init_mut() }.alloc_copy(original, bytes, align, offset, semantics),
            CopySelector::Unused => unreachable!()
        }
    }

    pub fn post_copy(&mut self, object: ObjectReference, bytes: usize, semantics: CopySemantics) {
        object_forwarding::clear_forwarding_bits::<VM>(object);
        if semantics.is_mature() {
            if self.config.constraints.needs_log_bit {
                VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.mark_as_unlogged::<VM>(object, Ordering::SeqCst);
            } else {
                unimplemented!("Mature copy is used but the plan does not use unlogged bit");
            }
        }
        match self.config.copy_mapping[semantics] {
            CopySelector::CopySpace(index) => unsafe { self.copy[index as usize].assume_init_mut() }.post_copy(object, bytes, semantics),
            CopySelector::Immix(index) => unsafe { self.immix[index as usize].assume_init_mut() }.post_copy(object, bytes, semantics),
            CopySelector::Unused => unreachable!()
        }
    }

    pub fn prepare(&mut self) {
        for (_, selector) in self.config.copy_mapping.iter() {
            match selector {
                CopySelector::CopySpace(index) => unsafe { self.copy[*index as usize].assume_init_mut() }.prepare(),
                CopySelector::Immix(index) => unsafe { self.immix[*index as usize].assume_init_mut() }.prepare(),
                CopySelector::Unused => {}
            }
        }
    }

    pub fn release(&mut self) {
        for (_, selector) in self.config.copy_mapping.iter() {
            match selector {
                CopySelector::CopySpace(index) => unsafe { self.copy[*index as usize].assume_init_mut() }.release(),
                CopySelector::Immix(index) => unsafe { self.immix[*index as usize].assume_init_mut() }.release(),
                CopySelector::Unused => {}
            }
        }
    }

    pub fn new(worker_tls: VMWorkerThread, plan: &'static dyn Plan<VM = VM>, config: CopyConfig, space_mapping: &[(CopySelector, &'static dyn Space<VM>)]) -> Self {
        let mut ret = GCWorkerCopyContext {
            copy: unsafe { MaybeUninit::uninit().assume_init() },
            immix: unsafe { MaybeUninit::uninit().assume_init() },
            config
        };

        for &(selector, space) in space_mapping.iter() {
            match selector {
                CopySelector::CopySpace(index) => {
                    ret.copy[index as usize].write(CopySpaceCopyContext::new(worker_tls, plan, space.downcast_ref::<CopySpace<VM>>().unwrap()));
                },
                CopySelector::Immix(index) => {
                    ret.immix[index as usize].write(ImmixCopyContext::new(worker_tls, plan, space.downcast_ref::<ImmixSpace<VM>>().unwrap()));
                }
                CopySelector::Unused => unreachable!(),
            }
        }

        ret
    }

    pub fn new_non_copy() -> Self {
        GCWorkerCopyContext {
            copy: unsafe { MaybeUninit::uninit().assume_init() },
            immix: unsafe { MaybeUninit::uninit().assume_init() },
            config: CopyConfig {
                copy_mapping: EnumMap::default(),
                // enum_map! {
                //     CopySemantics::DefaultCopy => CopySelector::Unused,
                //     CopySemantics::DefaultCompact => CopySelector::Unused,
                //     CopySemantics::PromoteMature => CopySelector::Unused,
                //     CopySemantics::NurseryCopy => CopySelector::Unused,
                //     CopySemantics::NurseryCompact => CopySelector::Unused,
                //     CopySemantics::MatureCopy => CopySelector::Unused,
                //     CopySemantics::MatureCompact => CopySelector::Unused,
                // },
                constraints: &crate::plan::DEFAULT_PLAN_CONSTRAINTS,
            }
        }
    }
}

impl<VM: VMBinding> GCWorkerLocal for GCWorkerCopyContext<VM> {}

#[derive(Clone, Copy, Enum, Debug)]
pub enum CopySemantics {
    DefaultCopy,
    DefaultCompact,
    NurseryCopy,
    NurseryCompact,
    PromoteMature,
    MatureCopy,
    MatureCompact,
}

impl CopySemantics {
    pub fn is_mature(&self) -> bool {
        match self {
            CopySemantics::PromoteMature | CopySemantics::MatureCompact | CopySemantics::MatureCopy => true,
            _ => false,
        }
    }

    pub fn is_compact(&self) -> bool {
        match self {
            CopySemantics::DefaultCompact | CopySemantics::NurseryCompact | CopySemantics::MatureCompact => true,
            _ => false,
        }
    }
}

#[repr(C, u8)]
#[derive(Copy, Clone, Debug)]
pub enum CopySelector {
    CopySpace(u8),
    Immix(u8),
    Unused,
}

impl std::default::Default for CopySelector {
    fn default() -> Self {
        CopySelector::Unused
    }
}