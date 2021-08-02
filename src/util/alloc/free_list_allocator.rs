use std::{mem::size_of, ops::BitAnd};

use atomic::Ordering;

use crate::{Plan, policy::{marksweepspace::{MarkSweepSpace, metadata::{set_alloc_bit, unset_alloc_bit_unsafe}}, space::Space}, util::{Address, OpaquePointer, VMThread, VMWorkerThread, constants::{LOG_BYTES_IN_PAGE}, metadata::{MetadataSpec, compare_exchange_metadata, load_metadata, store_metadata}}, vm::VMBinding};

use super::Allocator;

pub(crate) const BYTES_IN_BLOCK: usize = 1 << LOG_BYTES_IN_BLOCK;
pub(crate) const LOG_BYTES_IN_BLOCK: usize = 16;
const MI_BIN_HUGE: usize = 73;
const MI_INTPTR_SHIFT: usize = 3;
const MI_INTPTR_SIZE: usize = 1 << MI_INTPTR_SHIFT;
pub const MI_LARGE_OBJ_SIZE_MAX: usize = 1 << 21;
const MI_LARGE_OBJ_WSIZE_MAX: usize = MI_LARGE_OBJ_SIZE_MAX/MI_INTPTR_SIZE;
const MI_INTPTR_BITS: usize = MI_INTPTR_SIZE*8;
const MI_BIN_FULL: usize = MI_BIN_HUGE + 1;

// mimalloc init.c:46
pub(crate) const BLOCK_QUEUES_EMPTY: [BlockQueue; 74] = [
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
                    &MetadataSpec::OnSide(self.space.get_free_metadata_spec()), 
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
            &MetadataSpec::OnSide(self.space.get_free_metadata_spec()),
            unsafe{block.to_object_reference()}, 
            next_cell.as_usize(), None, 
            None
        );

        // set allocation bit
        set_alloc_bit(unsafe { free_list.to_object_reference() });

        free_list
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        // first, do frees from other threads (todo)


        // try to find an existing block with free cells
        let block = self.find_free_block(size);

        // _mi_page_malloc
        let free_list = unsafe {
            Address::from_usize(
                load_metadata::<VM>(
                    &MetadataSpec::OnSide(self.space.get_free_metadata_spec()), 
                    block.to_object_reference(), 
                    None, 
                    None,
                )
            )
        };
        
        // update free list
        let next_cell = unsafe { free_list.load::<Address>() };
        store_metadata::<VM>(
            &MetadataSpec::OnSide(self.space.get_free_metadata_spec()),
            unsafe{block.to_object_reference()}, 
            next_cell.as_usize(), None, 
            None
        ); 

        // set allocation bit
        set_alloc_bit(unsafe { free_list.to_object_reference() });

        free_list
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

    #[inline]
    pub fn get_free_list(&self, block: Address) -> Address {
        unsafe {
            Address::from_usize(
                load_metadata::<VM>(
                    &MetadataSpec::OnSide(self.space.get_free_metadata_spec()), 
                    block.to_object_reference(), 
                    None, 
                    None,
                )
            )
        }
    }

    #[inline]
    pub fn set_free_list(&self, block: Address, free_list: Address) {
        store_metadata::<VM>(
            &MetadataSpec::OnSide(self.space.get_free_metadata_spec()),
            unsafe{block.to_object_reference()}, 
            free_list.as_usize(),
            None, 
            None
        );
    }

    #[inline]
    pub fn get_local_free_list(&self, block: Address) -> Address {
        unsafe {
            Address::from_usize(
                load_metadata::<VM>(
                    &MetadataSpec::OnSide(self.space.get_local_free_metadata_spec()), 
                    block.to_object_reference(), 
                    None, 
                    None,
                )
            )
        }
    }
    
    #[inline]
    pub fn set_local_free_list(&self, block: Address, local_free: Address) {
        store_metadata::<VM>(
            &MetadataSpec::OnSide(self.space.get_local_free_metadata_spec()),
            unsafe{block.to_object_reference()}, 
            local_free.as_usize(),
            None, 
            None
        );
    }

    #[inline]
    pub fn get_thread_free_list(&self, block: Address) -> Address {
        unsafe {
            Address::from_usize(
                load_metadata::<VM>(
                    &MetadataSpec::OnSide(self.space.get_thread_free_metadata_spec()), 
                    block.to_object_reference(), 
                    None, 
                    Some(Ordering::SeqCst),
                )
            )
        }
    }
    
    #[inline]
    pub fn cas_thread_free_list(&self, block: Address, old_thread_free: Address, new_thread_free: Address) -> bool {
        compare_exchange_metadata::<VM>(
            &MetadataSpec::OnSide(self.space.get_thread_free_metadata_spec()),
            unsafe{block.to_object_reference()}, 
            old_thread_free.as_usize(),
            new_thread_free.as_usize(),
            None,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
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
                    &MetadataSpec::OnSide(self.space.get_free_metadata_spec()), 
                    block.to_object_reference(), 
                    None, 
                    None,
                )
            )
        };

        if free_list != unsafe { Address::zero() } {
            // first block is available
            return block; // fast path
        }

        if unsafe { free_list == Address::zero() } {
            // first block is exhausted, get next block and try again
            block = loop {
                if unsafe { block == Address::zero() } {
                    // no more blocks
                    break { self.acquire_block_for_size(size) }
                }
                let next_block = unsafe {
                    Address::from_usize(
                        load_metadata::<VM>(
                            &MetadataSpec::OnSide(self.space.get_next_metadata_spec()), 
                            block.to_object_reference(),
                            None,
                            None,
                        )
                    )
                };

                self.block_free_collect(block);
                
                let free_list = unsafe {
                    Address::from_usize(
                        load_metadata::<VM>(
                            &MetadataSpec::OnSide(self.space.get_free_metadata_spec()), 
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
                block = next_block;            
            }
        };
        block

    }

    pub fn block_thread_free_collect(&self, block: Address) {
        let free_list = self.get_free_list(block);

        let mut success = false;
        let mut thread_free = unsafe { Address::zero() };
        while !success {
            thread_free = self.get_thread_free_list(block);
            success = self.cas_thread_free_list(block, thread_free, unsafe { Address::zero() });
        }
        
        // no more CAS needed
        // futher frees to the thread free list will be done from a new empty list
        if unsafe { free_list == Address::zero() } {
            self.set_free_list(block, thread_free);
        } else {
            let mut tail = thread_free;
            unsafe { 
                while tail != Address::zero() {
                    tail = tail.load::<Address>();
                }
                tail.store(free_list);
            }
            self.set_free_list(block, thread_free);
        }
    }

    pub fn block_free_collect(&self, block: Address) {

        let free_list = self.get_free_list(block);

        // first, other threads
        self.block_thread_free_collect(block);

        // same thread
        let local_free = self.get_local_free_list(block);

        if unsafe { free_list == Address::zero() } {
            self.set_free_list(block, local_free);
        } else {
            let mut tail = local_free;
            unsafe { 
                while tail != Address::zero() {
                    tail = tail.load::<Address>();
                }
                tail.store(free_list);
            }
            self.set_free_list(block, local_free);
        }
        unsafe {
            self.set_local_free_list(block, Address::zero())
        }
    }

    pub fn acquire_block_for_size(&mut self, size: usize) -> Address {
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
        let next = block_queue.first;
        block_queue.first = block;
        store_metadata::<VM>(&MetadataSpec::OnSide(self.space.get_next_metadata_spec()), unsafe{ block.to_object_reference() }, next.as_usize(), None, None);
        store_metadata::<VM>(&MetadataSpec::OnSide(self.space.get_free_metadata_spec()), unsafe{ block.to_object_reference() }, final_cell.as_usize(), None, None);
        store_metadata::<VM>(&MetadataSpec::OnSide(self.space.get_size_metadata_spec()), unsafe{ block.to_object_reference() }, size, None, None);
        self.set_local_free_list(block, unsafe{Address::zero()});
        self.store_block_tls(block);
        trace!("Constructed free list for block starting at {}", block);
        block
    }

    pub fn get_block(addr: Address) -> Address {
        let block = unsafe { Address::from_usize(addr.bitand(!0xFFFF as usize)) };
        block
    }

    
    fn acquire_block(&self) -> Address {
        // acquire 64kB block
        let block = self.space.acquire(self.tls, BYTES_IN_BLOCK >> LOG_BYTES_IN_PAGE);
        self.space.active_blocks.lock().unwrap().insert(block);
        block
    }

    pub fn store_block_tls(&self, block: Address) {
        let tls = unsafe { std::mem::transmute::<OpaquePointer, usize>(self.tls.0) };
        store_metadata::<VM>(
            &MetadataSpec::OnSide(self.space.get_tls_metadata_spec()),
            unsafe{block.to_object_reference()}, 
            tls,
            None, 
            None
        );
    }

    pub fn reset(&mut self) {
        trace!("reset");
        // zero free lists
        self.blocks = BLOCK_QUEUES_EMPTY.to_vec();
    }

    pub fn rebind(&mut self, space: &'static MarkSweepSpace<VM>,) {
        trace!("rebind");
        self.reset();
        self.space = space;
    }


    // pub fn free(space: &'static MarkSweepSpace<VM>, addr: Address, tls: VMWorkerThread) {

    //     let block = FreeListAllocator::<VM>::get_owning_block(addr);
    //     let block_tls = space.load_block_tls(block);

    //     if tls.0.0 == block_tls {
    //         // same thread that allocated
    //         let local_free = unsafe {
    //             Address::from_usize(
    //                 load_metadata::<VM>(
    //                     MetadataSpec::OnSide(space.get_local_free_metadata_spec()), 
    //                     block.to_object_reference(), 
    //                     None, 
    //                     None,
    //                 )
    //             )
    //         };
    //         unsafe {
    //             addr.store(local_free);
    //         }
    //         store_metadata::<VM>(
    //             MetadataSpec::OnSide(space.get_free_metadata_spec()),
    //             unsafe{block.to_object_reference()}, 
    //             addr.as_usize(), None, 
    //             None
    //         );
    //     } else {
    //         // different thread to allocator
    //         let mut success = false;
    //         while !success {
    //             let thread_free = unsafe {
    //                 Address::from_usize(
    //                     load_metadata::<VM>(
    //                         MetadataSpec::OnSide(space.get_thread_free_metadata_spec()), 
    //                         block.to_object_reference(), 
    //                         None, 
    //                         Some(Ordering::SeqCst),
    //                     )
    //                 )
    //             };
    //             unsafe {
    //                 addr.store(thread_free);
    //             }
    //             success = compare_exchange_metadata::<VM>(
    //                 MetadataSpec::OnSide(space.get_free_metadata_spec()),
    //                 unsafe{block.to_object_reference()}, 
    //                 thread_free.as_usize(), 
    //                 addr.as_usize(), 
    //                 None,
    //                 Ordering::SeqCst,
    //                 Ordering::SeqCst, //?
    //             );
    //         }
    //     }
        

    //     // unset allocation bit
    //     unset_alloc_bit(unsafe { addr.to_object_reference() });

    // }


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