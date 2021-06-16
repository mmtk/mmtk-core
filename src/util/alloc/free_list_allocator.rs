use std::{collections::{HashMap, LinkedList}};

use crate::{Plan, policy::marksweepspace::MarkSweepSpace, util::{Address, VMThread}, vm::VMBinding};

use super::Allocator;

pub struct FreeListAllocator<VM: VMBinding> {
  pub tls: VMThread,
  space: MarkSweepSpace<VM>,
  plan: &'static dyn Plan<VM = VM>,
  available_blocks: BlockList,
  exhausted_blocks: BlockList,
}

type SizeClass = usize;
type BlockList = HashMap<SizeClass, LinkedList<LinkedList<Address>>>;

impl<VM: VMBinding> Allocator<VM> for FreeListAllocator<VM> {
    fn get_tls(&self) -> VMThread {
        self.tls
    }

    fn get_space(&self) -> &'static dyn crate::policy::space::Space<VM> {
        &self.space
    }

    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        todo!()
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        todo!()
    }
}

impl<VM: VMBinding> FreeListAllocator<VM> {
    fn get_size_class(size: usize) -> SizeClass {
        todo!()
    }

    fn make_free_list(block: Address, size_class: SizeClass) {
        todo!()
    }
}
