use std::{collections::{LinkedList}, mem::size_of, ops::BitAnd, ptr::null};

use crate::{Plan, policy::{marksweepspace::MarkSweepSpace, space::Space}, util::{Address, VMThread, constants::{LOG_BYTES_IN_PAGE}, heap::layout::vm_layout_constants::BYTES_IN_CHUNK}, vm::VMBinding};

use super::Allocator;

const BYTES_IN_BLOCK: usize = 1 << LOG_BYTES_IN_BLOCK;
const LOG_BYTES_IN_BLOCK: usize = 16;

pub struct FreeListAllocator<VM: VMBinding> {
    pub tls: VMThread,
    space: &'static MarkSweepSpace<VM>,
    plan: &'static dyn Plan<VM = VM>,
    blocks_direct: Vec<Address>
  }
  
#[derive(Clone, Copy, Debug)]
struct BlockData {
    next: Address,
    free: Address,
    size: usize, // change to metadata ?
}

unsafe impl<VM: VMBinding> Send for FreeListAllocator<VM> {}



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
        trace!("Free list allocator: allocation request for {} bytes", size);
        let block_data_address = self.blocks_direct[if size < 129 { size - 1 } else { 128 }];
        if unsafe { block_data_address == Address::zero() } {
            // no block for this size, go to slow path
            return self.alloc_slow_once(size, align, offset);
        }
        trace!("Free List Allocator: found block for size {}, block data = {:?}", size, unsafe{block_data_address.load::<BlockData>()});
        let cell = FreeListAllocator::<VM>::attempt_alloc_to_block(block_data_address);
        if unsafe { cell == Address::zero() } {
            // no cells available for this size, go to slow path
            return self.alloc_slow_once(size, align, offset);
        }
        trace!("Free list allocator: fast alloc to {}", cell);
        cell
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let block = self.acquire_block_for_size(size + size_of::<Address>());
        let block_data_address = block + BYTES_IN_BLOCK - size_of::<BlockData>();
        let cell = FreeListAllocator::<VM>::attempt_alloc_to_block(block_data_address);
        trace!("Free list allocator: slow alloc to {}", cell);
        cell
    }
}

impl<VM: VMBinding> FreeListAllocator<VM> {


    pub fn new(
        tls: VMThread,
        space: &'static MarkSweepSpace<VM>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        let mut allocator = FreeListAllocator {
            tls,
            space,
            plan,
            blocks_direct: vec![unsafe{ Address::zero() }; 129],
        };
        allocator
    }

    pub fn acquire_block_for_size(&mut self, size: usize) -> Address {
        let size = if size < 129 { size } else { 1 << 14 };
        let block = self.acquire_block();
        let block_data_address = block + BYTES_IN_BLOCK - size_of::<BlockData>();
        let mut old_cell = block;
        let mut new_cell = block + size;
        let final_cell = loop {
            unsafe {
                (new_cell + size - size_of::<Address>()).store::<Address>(old_cell);
                // trace!("Store {} at {}", old_cell, new_cell + size - size_of::<Address>());
            }
            old_cell = new_cell;
            new_cell = old_cell + size;
            if new_cell + size >= block_data_address {break old_cell};
        };
        let block_data = BlockData {
            next: self.blocks_direct[ if size < 129 { size - 1 } else { 128 }],
            free: final_cell,
            size: if size < 129 { size } else { 1 << 14 },
        };
        unsafe {
            trace!("Acquired block for size {}, block data = {:?}", size, block_data);
            block_data_address.store::<BlockData>(block_data);
        };

        self.blocks_direct[if size < 129 { size - 1 } else { 128 }] = block_data_address;

        trace!("Constructed free list for block starting at {}", block);
        block
    }

    fn attempt_alloc_to_block(block_data_address: Address) -> Address {
        // return cell if found, cell in following blocks if found, else return zero
        let mut block_data = unsafe { block_data_address.load::<BlockData>() };
        let cell = block_data.free;
        if unsafe { cell == Address::zero() } {
            // block is exhausted, get next block and try again
            let block_data_address = block_data.next;
            if unsafe { block_data_address == Address::zero() } {
                // no more blocks, return zero
                return unsafe { Address::zero() };
            }
            let block_data = unsafe { block_data_address.load::<BlockData>() };
            return FreeListAllocator::<VM>::attempt_alloc_to_block(block_data_address);
        };
        let next_cell = unsafe { (cell + block_data.size - size_of::<Address>()).load::<Address>() };
        // trace!("Load {} from {}", next_cell, (cell + block_data.size - size_of::<Address>()));
        block_data.free = next_cell;
        unsafe { block_data_address.store::<BlockData>(block_data) };
        cell
    }

    fn get_owning_block(addr: Address) -> Address {
        unsafe { Address::from_usize(addr.bitand(0x10000 as usize)) }
    }

    
    fn acquire_block(&self) -> Address {
        // acquire 64kB block
        let a = self.space.acquire(self.tls, BYTES_IN_CHUNK >> LOG_BYTES_IN_PAGE);//BYTES_IN_BLOCK >> LOG_BYTES_IN_PAGE);
        a
    }

    pub fn return_block(&self) {
        // return freed 64kB block
        todo!()
    }
}