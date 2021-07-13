use std::{mem::size_of, ops::BitAnd};

use crate::{Plan, policy::{marksweepspace::MarkSweepSpace, space::Space}, util::{Address, VMThread, constants::{LOG_BYTES_IN_PAGE}, metadata::{MetadataSpec, load_metadata, side_metadata::SideMetadataSpec, store_metadata}}, vm::VMBinding};

use super::Allocator;

pub(crate) const BYTES_IN_BLOCK: usize = 1 << LOG_BYTES_IN_BLOCK;
const LOG_BYTES_IN_BLOCK: usize = 16;
const MI_BIN_HUGE: usize = 73;
const MI_INTPTR_SHIFT: usize = 3;
const MI_INTPTR_SIZE: usize = 1 << MI_INTPTR_SHIFT;
pub const MI_LARGE_OBJ_SIZE_MAX: usize = 1 << 21;
const MI_LARGE_OBJ_WSIZE_MAX: usize = MI_LARGE_OBJ_SIZE_MAX/MI_INTPTR_SIZE;
const MI_INTPTR_BITS: usize = MI_INTPTR_SIZE*8;
const MI_BIN_FULL: usize = MI_BIN_HUGE + 1;

// mimalloc init.c:46
const BLOCK_QUEUES_EMPTY: [BlockQueue; 74] = [
    BlockQueue::new(     1*4),
    BlockQueue::new(     1*4), BlockQueue::new(     2*4), BlockQueue::new(     3*4), BlockQueue::new(     4*4), BlockQueue::new(     5*4), BlockQueue::new(     6*4), BlockQueue::new(     7*4), BlockQueue::new(     8*4), /* 8 */ 
    BlockQueue::new(    10*4), BlockQueue::new(    12*4), BlockQueue::new(    14*4), BlockQueue::new(    16*4), BlockQueue::new(    20*4), BlockQueue::new(    24*4), BlockQueue::new(    28*4), BlockQueue::new(    32*4), /* 16 */ 
    BlockQueue::new(    40*4), BlockQueue::new(    48*4), BlockQueue::new(    56*4), BlockQueue::new(    64*4), BlockQueue::new(    80*4), BlockQueue::new(    96*4), BlockQueue::new(   112*4), BlockQueue::new(   128*4), /* 24 */ 
    BlockQueue::new(   160*4), BlockQueue::new(   192*4), BlockQueue::new(   224*4), BlockQueue::new(   256*4), BlockQueue::new(   320*4), BlockQueue::new(   384*4), BlockQueue::new(   448*4), BlockQueue::new(   512*4), /* 32 */ 
    BlockQueue::new(   640*4), BlockQueue::new(   768*4), BlockQueue::new(   896*4), BlockQueue::new(  1024*4), BlockQueue::new(  1280*4), BlockQueue::new(  1536*4), BlockQueue::new(  1792*4), BlockQueue::new(  2048*4), /* 40 */ 
    BlockQueue::new(  2560*4), BlockQueue::new(  3072*4), BlockQueue::new(  3584*4), BlockQueue::new(  4096*4), BlockQueue::new(  5120*4), BlockQueue::new(  6144*4), BlockQueue::new(  7168*4), BlockQueue::new(  8192*4), /* 48 */ 
    BlockQueue::new( 10240*4), BlockQueue::new( 12288*4), BlockQueue::new( 14336*4), BlockQueue::new( 16384*4), BlockQueue::new( 20480*4), BlockQueue::new( 24576*4), BlockQueue::new( 28672*4), BlockQueue::new( 32768*4), /* 56 */ 
    BlockQueue::new( 40960*4), BlockQueue::new( 49152*4), BlockQueue::new( 57344*4), BlockQueue::new( 65536*4), BlockQueue::new( 81920*4), BlockQueue::new( 98304*4), BlockQueue::new(114688*4), BlockQueue::new(131072*4), /* 64 */ 
    BlockQueue::new(163840*4), BlockQueue::new(196608*4), BlockQueue::new(229376*4), BlockQueue::new(262144*4), BlockQueue::new(327680*4), BlockQueue::new(393216*4), BlockQueue::new(458752*4), BlockQueue::new(524288*4), /* 72 */ 
    BlockQueue::new(MI_LARGE_OBJ_WSIZE_MAX + 1  /* 655360, Huge queue */),
];


pub struct FreeListAllocator<VM: VMBinding> {
    pub tls: VMThread,
    space: &'static MarkSweepSpace<VM>,
    plan: &'static dyn Plan<VM = VM>,
    blocks: Vec<BlockQueue>, // = pages in mimalloc
  }
  #[derive(Clone, Copy, Debug)]
pub struct BlockQueue {
    first: Address,
    size: usize,
}

impl BlockQueue {
    const fn new(size: usize) -> BlockQueue {
        BlockQueue {
            first: unsafe { Address::zero() },
            size,
        }
    }
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
        // see mi_heap_malloc_small
        assert!(size < BYTES_IN_BLOCK, "Alloc request for {} bytes is too big.", size);
        // eprintln!("alloc {} bytes", size);

        // _mi_heap_get_free_small_page
        let bin = FreeListAllocator::<VM>::mi_bin(size);
        // eprintln!("Free List Allocator: allocation request for {} bytes, fits in bin #{}", size, bin);
        let block_queue = &self.blocks[bin as usize];
        let block = block_queue.first;
        if unsafe { block == Address::zero() } {
            // no block for this size, go to slow path
            return self.alloc_slow_once(size, align, offset);
        }

        // _mi_page_malloc
        let free_list = unsafe {
            Address::from_usize(
                load_metadata::<VM>(
                    MetadataSpec::OnSide(self.get_free_metadata_spec()), 
                    block.to_object_reference(), 
                    None, 
                    None,
                )
            )
        };

        if free_list == unsafe { Address::zero() } {
            // first block has no empty cells, go to slow path
            return self.alloc_slow_once(size, align, offset);
        }
        
        // update free list
        let next_cell = unsafe { free_list.load::<Address>() };
        store_metadata::<VM>(
            MetadataSpec::OnSide(self.get_free_metadata_spec()),
            unsafe{block.to_object_reference()}, 
            next_cell.as_usize(), None, 
            None
        );
        free_list

        // // This should be in the slow path!!!
        // let cell = self.attempt_alloc_to_block(block);
        // if unsafe { cell == Address::zero() } {
        //     // eprintln!("!! go to slow path");
        //     // no cells available for this size, go to slow path
        //     return self.alloc_slow_once(size, align, offset);
        // }
        // // eprintln!("Free list allocator: fast alloc {} bytes to {}", size, cell);
        // cell
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        // first, do freeing (not yet)

        // try to find an existing block with free cells
        let block = self.find_free_block(size);

        // _mi_page_malloc
        let free_list = unsafe {
            Address::from_usize(
                load_metadata::<VM>(
                    MetadataSpec::OnSide(self.get_free_metadata_spec()), 
                    block.to_object_reference(), 
                    None, 
                    None,
                )
            )
        };

        if free_list == unsafe { Address::zero() } {
            // first block has no empty cells, go to slow path
            return self.alloc_slow_once(size, align, offset);
        }
        
        // update free list
        let next_cell = unsafe { free_list.load::<Address>() };
        store_metadata::<VM>(
            MetadataSpec::OnSide(self.get_free_metadata_spec()),
            unsafe{block.to_object_reference()}, 
            next_cell.as_usize(), None, 
            None
        );
        free_list
        // // none exist, allocate a new block
        // let block = self.acquire_block_for_size(size);
        // // eprintln!("Acquired block");
        // let cell = self.attempt_alloc_to_block(block);
        // // eprintln!("Free list allocator: slow alloc {} bytes to {}", size, cell);
    }
}

impl<VM: VMBinding> FreeListAllocator<VM> {
    pub fn new(
        tls: VMThread,
        space: &'static MarkSweepSpace<VM>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        FreeListAllocator {
            tls,
            space,
            plan,
            blocks: BLOCK_QUEUES_EMPTY.to_vec(),
        }
    }

    pub fn get_next_metadata_spec(&self) -> SideMetadataSpec {
        self.space.common.metadata.local[0]
    }

    pub fn get_free_metadata_spec(&self) -> SideMetadataSpec {
        self.space.common.metadata.local[1]
    }

    pub fn get_size_metadata_spec(&self) -> SideMetadataSpec {
        self.space.common.metadata.local[2]
    }

    pub fn find_free_block(&mut self, size: usize) -> Address {
        // mi_find_free_page

        let bin = FreeListAllocator::<VM>::mi_bin(size);
        let block_queue = &self.blocks[bin as usize];
        let mut block = block_queue.first;

        // block queue is empty
        if unsafe { block == Address::zero() } {
            return self.acquire_block_for_size(size);
        }

        let free_list = unsafe {
            Address::from_usize(
                load_metadata::<VM>(
                    MetadataSpec::OnSide(self.get_free_metadata_spec()), 
                    block.to_object_reference(), 
                    None, 
                    None,
                )
            )
        };

        if free_list != unsafe { Address::zero() } {
            // first block is available
            return block;
        }

        if unsafe { free_list == Address::zero() } {
            // first block is exhausted, get next block and try again
            block = loop {
                block = unsafe {
                    Address::from_usize(
                        load_metadata::<VM>(
                            MetadataSpec::OnSide(self.get_next_metadata_spec()), 
                            block.to_object_reference(),
                            None,
                            None,
                        )
                    )
                };
                if unsafe { block == Address::zero() } {
                    // no more blocks
                    break { self.acquire_block_for_size(size) }
                }
                
                let free_list = unsafe {
                    Address::from_usize(
                        load_metadata::<VM>(
                            MetadataSpec::OnSide(self.get_free_metadata_spec()), 
                            block.to_object_reference(), 
                            None, 
                            None,
                        )
                    )
                };
                if unsafe { free_list != Address::zero() } {
                    // found a free block
                    break { block }
                }
            
            }
        };
        block

    }


    pub fn acquire_block_for_size(&mut self, size: usize) -> Address {
        // eprintln!("Acquire block for size {}", size);
        let block = self.acquire_block();

        // construct free list
        let block_end = block + BYTES_IN_BLOCK;
        let mut old_cell = unsafe { Address::zero() };
        let mut new_cell = block;
        let block_queue = self.blocks.get_mut(FreeListAllocator::<VM>::mi_bin(size) as usize).unwrap();
        trace!("Asked for size {}, make free list with size {}", size, block_queue.size);
        assert!(size <= block_queue.size);
        let final_cell = loop {
            unsafe {
                new_cell.store::<Address>(old_cell);
                // trace!("Store {} at {}", old_cell, new_cell);
            }
            old_cell = new_cell;
            new_cell = old_cell + block_queue.size;
            if new_cell + block_queue.size >= block_end {break old_cell};
        };
        // unsafe{block.store::<Address>(Address::zero())};
        // trace!("Store {} at {}", old_cell, new_cell);
        let next = block_queue.first;
        block_queue.first = block;
        store_metadata::<VM>(MetadataSpec::OnSide(self.get_next_metadata_spec()), unsafe{ block.to_object_reference() }, next.as_usize(), None, None);
        store_metadata::<VM>(MetadataSpec::OnSide(self.get_free_metadata_spec()), unsafe{ block.to_object_reference() }, final_cell.as_usize(), None, None);
        store_metadata::<VM>(MetadataSpec::OnSide(self.get_size_metadata_spec()), unsafe{ block.to_object_reference() }, size, None, None);
        trace!("Constructed free list for block starting at {}", block);
        // unreachable!();
        block
    }

    fn get_owning_block(addr: Address) -> Address {
        unsafe { Address::from_usize(addr.bitand(0x10000 as usize)) }
    }

    
    fn acquire_block(&self) -> Address {
        // acquire 64kB block
        let a = self.space.acquire(self.tls, BYTES_IN_BLOCK >> LOG_BYTES_IN_PAGE);
        a
    }

    pub fn return_block(&self) {
        // return freed 64kB block
        todo!()
    }


    fn mi_wsize_from_size(size: usize) -> usize {
        // Align a byte size to a size in machine words
        // i.e. byte size == `wsize*sizeof(void*)`
        // adapted from _mi_wsize_from_size in mimalloc
        (size + size_of::<u32>() - 1) / size_of::<u32>()
    }

    fn mi_bin(size: usize) -> u8 {
        // adapted from _mi_bin in mimalloc
        let mut wsize: usize = FreeListAllocator::<VM>::mi_wsize_from_size(size);
        let bin: u8;
        if wsize <= 1 {
            bin = 1;
        } else if wsize <= 8 {
            bin = wsize as u8;
            // bin = ((wsize + 1) & !1) as u8; // round to double word sizes
        } else if wsize > MI_LARGE_OBJ_WSIZE_MAX {
            // bin = MI_BIN_HUGE;
            panic!(); // this should not be reached, because I'm sending objects bigger than this to the immortal space
        } else {
            wsize -= 1;
            let b= (MI_INTPTR_BITS - 1 - (u64::leading_zeros(wsize as u64)) as usize) as u8;  // note: wsize != 0
            bin = ((b << 2) + ((wsize >> (b - 2)) & 0x03) as u8) - 3;
        }
        bin
      }
}