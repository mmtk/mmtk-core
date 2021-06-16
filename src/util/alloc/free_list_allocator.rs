use std::{collections::{HashMap, LinkedList}};

use atomic_traits::fetch::Add;

use crate::{Plan, policy::marksweepspace::MarkSweepSpace, util::{Address, VMThread}, vm::VMBinding};

use super::Allocator;

pub struct FreeListAllocator<VM: VMBinding> {
  pub tls: VMThread,
  space: &'static MarkSweepSpace<VM>,
  plan: &'static dyn Plan<VM = VM>,
  available_blocks: BlockList,
  exhausted_blocks: BlockList,
}

type SizeClass = usize;
type BlockList = HashMap<SizeClass, LinkedList<Block>>;
type Block = LinkedList<Address>;

impl<VM: VMBinding> Allocator<VM> for FreeListAllocator<VM> {
    fn get_tls(&self) -> VMThread {
        self.tls
    }

    fn get_space(&self) -> &'static dyn crate::policy::space::Space<VM> {
        self.space
    }

    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let size_class = FreeListAllocator::<VM>::get_size_class(size);

        // assumes block lists are initialised for all size classes
        assert!(self.available_blocks.contains_key(&size_class));
        assert!(self.exhausted_blocks.contains_key(&size_class));

        // available blocks for given size class
        let available_blocks = self.available_blocks.get_mut(&size_class).unwrap();

        if available_blocks.is_empty() {
            // no available blocks, go to slow path
            return self.alloc_slow_once(size, align, offset);
        }

        let block = available_blocks.front_mut().unwrap(); // first available block
        let address = FreeListAllocator::<VM>::alloc_to_block(block);
        unsafe {
            while address.eq(&Address::zero()) { // block is full
                self.exhausted_blocks.get_mut(&size_class).unwrap().push_back(available_blocks.pop_front().unwrap()); // move block to exhausted list
                let block = available_blocks.front_mut(); // next block
                if block.is_none() {
                    // all blocks exhausted, go to slow path
                    return self.alloc_slow_once(size, align, offset);
                }

                // next block
                let block = block.unwrap();
                let address = FreeListAllocator::<VM>::alloc_to_block(block);
            }
        }  
        address
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

    fn init_size_classes() {
        todo!()
    }

    fn alloc_to_block(block: &mut Block) -> Address {
        unsafe {
            match block.pop_front() {
                Some(cell) => cell,
                None => Address::zero(),
            }
        }
    }
}