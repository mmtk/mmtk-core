use std::{collections::{HashMap, LinkedList}};

use crate::{Plan, policy::{marksweepspace::MarkSweepSpace, space::Space}, util::{Address, VMThread, constants::{LOG_BYTES_IN_PAGE}}, vm::VMBinding};

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
        let size_class = FreeListAllocator::<VM>::get_size_class(size);

        // assumes block lists are initialised for all size classes
        assert!(self.available_blocks.contains_key(&size_class));
        assert!(self.exhausted_blocks.contains_key(&size_class));

        let block_start = self.acquire_block();
        let mut free_list = FreeListAllocator::<VM>::make_free_list(block_start, size_class);
        let address = free_list.pop_front().unwrap();
        self.available_blocks.get_mut(&size_class).unwrap().push_back(free_list);
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
        };
        free_list
    }

    fn init_size_classes(&mut self) {
        // TODO: multiple size classes

        self.available_blocks = HashMap::new();
        self.exhausted_blocks = HashMap::new();

        self.available_blocks.insert(1 << 13, LinkedList::new());
        self.exhausted_blocks.insert(1 << 13, LinkedList::new());
    }

    fn alloc_to_block(block: &mut Block) -> Address {
        unsafe {
            match block.pop_front() {
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
            available_blocks: HashMap::new(),
            exhausted_blocks: HashMap::new(),
        };
        allocator.init_size_classes();
        allocator
    }

    
    pub fn acquire_block(&self) -> Address {
        // acquire 64kB block
        self.space.acquire(self.tls, (1 << 13) >> LOG_BYTES_IN_PAGE)
    }

    pub fn return_block(&self) {
        // return freed 64kB block
        todo!()
    }
}