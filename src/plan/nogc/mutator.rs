use crate::plan::barriers::*;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::nogc::NoGC;
use crate::plan::AllocationSemantics as AllocationType;
use crate::util::alloc::allocators::{AllocatorSelector, Allocators};
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use enum_map::enum_map;
use enum_map::EnumMap;
use crate::scheduler::gc_works::*;
use crate::plan::{CopyContext, Plan};
use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::forwarding_word;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::MMTK;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map! {
        AllocationType::Default | AllocationType::Immortal | AllocationType::Code | AllocationType::ReadOnly | AllocationType::Los => AllocatorSelector::BumpPointer(0),
    };
}

pub fn nogc_mutator_noop<VM: VMBinding>(_mutator: &mut Mutator<NoGC<VM>>, _tls: OpaquePointer) {
    unreachable!();
}

pub fn create_nogc_mutator<VM: VMBinding>(
    mutator_tls: OpaquePointer,
    mmtk: &'static MMTK<VM>,
) -> Mutator<NoGC<VM>> {
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![(AllocatorSelector::BumpPointer(0), &mmtk.plan.nogc_space)],
        prepare_func: &nogc_mutator_noop,
        release_func: &nogc_mutator_noop,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, &mmtk.plan, &config.space_mapping),
        // barrier: box NoBarrier,
        barrier: box ObjectRememberingBarrier::<NoGCProcessEdges<VM>, super::global::NoGCImmortalSpace<VM>>::new(
            mmtk,
            &mmtk.plan.nogc_space,
        ),
        mutator_tls,
        config,
        plan: &mmtk.plan,
    }
}

#[derive(Default)]
pub struct NoGCProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<NoGCProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> ProcessEdgesWork for NoGCProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool) -> Self {
        Self {
            base: ProcessEdgesBase::new(edges),
            ..Default::default()
        }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        unreachable!()
    }
    #[inline]
    fn process_edge(&mut self, slot: Address) {
        unreachable!()
    }
}

impl<VM: VMBinding> Deref for NoGCProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for NoGCProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}