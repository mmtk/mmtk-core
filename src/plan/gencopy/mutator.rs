use crate::plan::mutator_context::{CommonMutatorContext, MutatorContext};
use crate::plan::Allocator as AllocationType;
use crate::util::alloc::Allocator;
use crate::util::alloc::BumpAllocator;
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::scheduler::MMTkScheduler;
use super::gc_works::GenCopyProcessModBuf;
use super::GenCopy;
use crate::vm::*;
use std::mem;
use crate::policy::space::Space;

#[repr(C)]
pub struct GenCopyMutator<VM: VMBinding> {
    ss: BumpAllocator<VM>,
    plan: &'static GenCopy<VM>,
    common: CommonMutatorContext<VM>,
    modbuf: Box<(Vec<ObjectReference>, Vec<Address>)>,
}

impl <VM: VMBinding> MutatorContext<VM> for GenCopyMutator<VM> {
    fn common(&self) -> &CommonMutatorContext<VM> {
        &self.common
    }

    fn prepare(&mut self, _tls: OpaquePointer) {
        // Do nothing
        self.flush_remembered_sets();
    }

    fn release(&mut self, _tls: OpaquePointer) {
        self.ss.rebind(Some(&self.plan.nursery));
        debug_assert!(self.modbuf.0.len() == 0);
        debug_assert!(self.modbuf.1.len() == 0);
    }

    fn alloc(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        allocator: AllocationType,
    ) -> Address {
        trace!(
            "MutatorContext.alloc({}, {}, {}, {:?})",
            size,
            align,
            offset,
            allocator
        );
        debug_assert!(
            self.ss.get_space().unwrap() as *const _ == &self.plan.nursery as *const _,
            "bumpallocator {:?} holds wrong space, ss.space: {:?}, tospace: {:?}",
            self as *const _,
            self.ss.get_space().unwrap() as *const _,
            self.plan.tospace() as *const _
        );
        match allocator {
            AllocationType::Default => self.ss.alloc(size, align, offset),
            _ => self.common.alloc(size, align, offset, allocator),
        }
    }

    fn post_alloc(
        &mut self,
        object: ObjectReference,
        _type: ObjectReference,
        _bytes: usize,
        allocator: AllocationType,
    ) {
        // debug_assert!(self.ss.get_space().unwrap() as *const _ == self.plan.tospace() as *const _);
        match allocator {
            AllocationType::Default => {}
            _ => self.common.post_alloc(object, _type, _bytes, allocator),
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        self.ss.tls
    }

    fn object_reference_write(&mut self, src: ObjectReference, slot: Address, value: ObjectReference) {
        if self.plan.copyspace0.address_in_space(slot) || self.plan.copyspace1.address_in_space(slot) {
            self.enqueue_edge(slot);
        }
    }

    fn record_modified_node(&mut self, obj: ObjectReference) {
        if !self.plan.nursery.in_space(obj) {
            // println!("record_modified_node {:?} .. {:?}", obj, if self.plan.copyspace0.in_space(obj) || self.plan.copyspace1.in_space(obj) {
            //     obj.to_address() + VM::VMObjectModel::get_current_size(obj)
            // } else {
            //     Address::ZERO
            // });
            self.enqueue_node(obj);
        }
    }
    fn record_modified_edge(&mut self, slot: Address) {
        if !self.plan.nursery.address_in_space(slot) {
            self.enqueue_edge(slot);
        }
    }

    fn flush_remembered_sets(&mut self) {
        let mut modified_nodes = vec![];
        mem::swap(&mut modified_nodes, &mut self.modbuf.0);
        let mut modified_edges = vec![];
        mem::swap(&mut modified_edges, &mut self.modbuf.1);
        self.plan.scheduler.closure_stage.add(GenCopyProcessModBuf {
            modified_nodes, modified_edges
        });
    }
}

impl <VM: VMBinding> GenCopyMutator<VM> {
    pub fn new(tls: OpaquePointer, plan: &'static GenCopy<VM>) -> Self {
        Self {
            ss: BumpAllocator::new(tls, Some(&plan.nursery), plan),
            plan,
            common: CommonMutatorContext::<VM>::new(tls, plan, &plan.common),
            modbuf: box (vec![], vec![]),
        }
    }

    fn enqueue_node(&mut self, obj: ObjectReference) {
        self.modbuf.0.push(obj);
        if self.modbuf.0.len() >= 512 {
            self.flush();
        }
    }

    fn enqueue_edge(&mut self, slot: Address) {
        self.modbuf.1.push(slot);
        if self.modbuf.1.len() >= 512 {
            self.flush();
        }
    }
}
