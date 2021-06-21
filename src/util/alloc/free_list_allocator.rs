use std::{collections::{HashMap, LinkedList}};

use crate::{Plan, policy::{marksweepspace::MarkSweepSpace, space::Space}, util::{Address, VMThread, constants::{LOG_BYTES_IN_PAGE}}, vm::VMBinding};

use super::Allocator;

pub struct FreeListAllocator<VM: VMBinding> {
    pub tls: VMThread,
    space: &'static MarkSweepSpace<VM>,
    plan: &'static dyn Plan<VM = VM>,
    available_blocks: Vec<LinkedList<(Block, LinkedList<Block>)>>,
    exhausted_blocks: Vec<LinkedList<(Block, LinkedList<Block>)>>,
  }
  
type SizeClass = usize;
type Block = Address;

impl<VM: VMBinding> Allocator<VM> for FreeListAllocator<VM> {
    fn get_tls(&self) -> VMThread {
        self.tls
    }

    fn get_space(&self) -> &'static dyn crate::policy::space::Space<VM> {
        self.space
    }

    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        // eprintln!("get plan {:?}", self.free_lists);
        self.plan
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        // eprintln!("alloc {:?}", self.free_lists);
        let size_class = FreeListAllocator::<VM>::get_size_class(size);

        // assumes block lists are initialised for all size classes
        // assert!(self.available_blocks.contains_key(&size_class));
        // assert!(self.exhausted_blocks.contains_key(&size_class));

        // available blocks for given size class
        let available_blocks = self.available_blocks.get(0).unwrap();
        if available_blocks.is_empty() {
            // no available blocks, go to slow path
            return self.alloc_slow_once(size, align, offset);
        }

        let found_cell = false;

        while !found_cell {
            let block = self.available_blocks.get_mut(0).unwrap().front_mut().unwrap();
            let (block, free_list) = block;
            let address = FreeListAllocator::<VM>::attempt_alloc_to_free_list(free_list);
            if address != unsafe {Address::zero()} {
                return address;
            }
            panic!("block is exhausted");
        };

        unreachable!();




        // let block = available_blocks.front_mut().unwrap();
        // let free_list = self.free_lists.get_mut(block);
        // let empty_free_list = free_list.is_none();
        // while empty_free_list {
        //     // block is exhausted
        //     self.exhausted_blocks.get_mut(&size_class).unwrap().push_back(*block); // move block to exhausted list
        //     self.free_lists.remove(block);
        //     available_blocks.pop_front();
        //     if available_blocks.is_empty() {
        //         // no available blocks, go to slow path
        //         return self.alloc_slow_once(size, align, offset);
        //     }
        //     let block = available_blocks.front_mut().unwrap();
        //     let free_list = self.free_lists.get_mut(block);
        //     empty_free_list = free_list.is_none();
        // }
        // let block = available_blocks.front_mut().unwrap();
        // let address = self.attempt_alloc_to_block(&block);
        // address
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        // eprintln!("alloc slow once {:?}", self.free_lists);
        let size_class = FreeListAllocator::<VM>::get_size_class(size);

        // assumes block lists are initialised for all size classes
        // assert!(self.available_blocks.contains_key(&size_class));
        // assert!(self.exhausted_blocks.contains_key(&size_class));
        let block_start = self.acquire_block();
        let mut free_list = FreeListAllocator::<VM>::make_free_list(block_start, size_class);
        let address = free_list.pop_front().unwrap();
        self.available_blocks.get_mut(0).unwrap().push_back((block_start, free_list));
        eprintln!("{}", address);
        address
    }
}

impl<VM: VMBinding> FreeListAllocator<VM> {
    fn get_size_class(size: usize) -> SizeClass {
        // TODO: multiple size classes
        // assuming largest is 8kB?
        1 << 13
    }

    fn make_free_list(block: Address, size_class: SizeClass) -> LinkedList<Address> {
        // cut 64kB block into cells for sizeclass
        // assumes fresh block, will later be required to recycle blocks based on liveness bitmap
        let mut cell = block;
        let mut free_list = LinkedList::new();
        let block_extent = unsafe { Address::from_usize(block.as_usize() + (1 << 16))}; //+64kB;
        while cell < block_extent {
            free_list.push_back(cell);
            cell = cell + size_class;
        };
        free_list
    }

    fn init_size_classes(&mut self) {
        // eprintln!("init size classes {:?}", self.free_lists);
        // TODO: multiple size classes

        self.available_blocks = vec![];
        self.exhausted_blocks = vec![];
        // self.free_lists = HashMap::new();

        self.available_blocks.insert(0, LinkedList::new());
        self.exhausted_blocks.insert(0, LinkedList::new());
    }

    fn attempt_alloc_to_free_list(free_list: &mut LinkedList<Address>) -> Address {
        unsafe {
            match free_list.pop_front() {
                Some(cell) => cell,
                None => Address::zero(),
            }
        }
    }

    pub fn new(
        tls: VMThread,
        space: &'static MarkSweepSpace<VM>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        let mut allocator = FreeListAllocator {
            tls,
            space,
            plan,
            available_blocks: vec![],
            exhausted_blocks: vec![],
        };
        allocator.init_size_classes();
        allocator
    }

    
    pub fn acquire_block(&self) -> Address {
        // acquire 64kB block
        let a = self.space.acquire(self.tls, (1 << 16) >> LOG_BYTES_IN_PAGE);
        a
    }

    pub fn return_block(&self) {
        // return freed 64kB block
        todo!()
    }
}