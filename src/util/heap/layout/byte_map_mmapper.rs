use super::Mmapper;
use ::util::{Address, ObjectReference};

use ::util::constants::*;
use std::fmt;
use std::sync::Mutex;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use util::heap::layout::vm_layout_constants::*;
use util::conversions::pages_to_bytes;

use super::mmapper::MMAP_CHUNK_BYTES;

use libc::*;
use std::mem::transmute;

const UNMAPPED: u8 = 0;
const MAPPED: u8 = 1;
const PROTECTED: u8 = 2;

const MMAP_NUM_CHUNKS: usize = if_then_else_usize!(LOG_BYTES_IN_ADDRESS_SPACE == 32,
                                                  1 << (LOG_BYTES_IN_ADDRESS_SPACE as usize - LOG_MMAP_CHUNK_BYTES),
                                                  1 << (33 - LOG_MMAP_CHUNK_BYTES));
pub const VERBOSE: bool = true;

pub struct ByteMapMmapper {
    lock: Mutex<()>,
    mapped: [AtomicU8; MMAP_NUM_CHUNKS],
}

impl fmt::Debug for ByteMapMmapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ByteMapMmapper({})", MMAP_NUM_CHUNKS)
    }
}

impl Mmapper for ByteMapMmapper {
    fn eagerly_mmap_all_spaces(&self, space_map: &[Address]) {
        unimplemented!()
    }

    fn mark_as_mapped(&self, start: Address, bytes: usize) {
        let start_chunk = Self::address_to_mmap_chunks_down(start);
        let end_chunk = Self::address_to_mmap_chunks_up(start + bytes);
        for i in start_chunk .. end_chunk + 1 {
            self.mapped[i].store(MAPPED, Ordering::Relaxed);
        }
    }

    fn ensure_mapped(&self, start: Address, pages: usize) {
        trace!("Calling ensure_mapped with start={:?} and {} pages", start, pages);
        let start_chunk = Self::address_to_mmap_chunks_down(start);
        let end_chunk = Self::address_to_mmap_chunks_up(start + pages_to_bytes(pages));

        for chunk in start_chunk..end_chunk {
            if self.mapped[chunk].load(Ordering::Relaxed) == MAPPED {
                continue;
            }

            let mmap_start = Self::mmap_chunks_to_address(chunk);
            let guard = self.lock.lock().unwrap();
//          trace!(mmapStart);
            // might have become MAPPED here
            if self.mapped[chunk].load(Ordering::Relaxed) == UNMAPPED {
                let mmap_ret = Address::from_mut_ptr(unsafe {
                    mmap(mmap_start.to_mut_ptr(), MMAP_CHUNK_BYTES,
                         PROT_READ | PROT_WRITE | PROT_EXEC,
                         MAP_ANON | MAP_PRIVATE | MAP_FIXED, -1, 0)
                });

                if mmap_ret != mmap_start {
                    drop(guard);
                    panic!("ensureMapped failed on address {}\n\
                           Can't get more space with mmap()", mmap_start);
                } else {
                    if VERBOSE {
                        trace!("mmap succeeded at chunk {}  {} with len = {}", chunk,
                               mmap_start, MMAP_CHUNK_BYTES);
                    }
                }
            }

            if self.mapped[chunk].load(Ordering::Relaxed) == PROTECTED {
                if unsafe { mprotect(mmap_start.to_mut_ptr(), MMAP_CHUNK_BYTES,
                                     PROT_READ | PROT_WRITE | PROT_EXEC) != 0 } {
                    drop(guard);
                    panic!("Mmapper.ensureMapped (unprotect) failed");
                } else {
                    if VERBOSE {
                        trace!("munprotect succeeded at chunk {}  {} with len = {}", chunk,
                               mmap_start, MMAP_CHUNK_BYTES);
                    }
                }
            }

            self.mapped[chunk].store(MAPPED, Ordering::Relaxed);
            drop(guard);
        }
    }

    /**
     * Return {@code true} if the given address has been mmapped
     *
     * @param addr The address in question.
     * @return {@code true} if the given address has been mmapped
     */
    fn address_is_mapped(&self, addr: Address) -> bool {
        let chunk = Self::address_to_mmap_chunks_down(addr);
        self.mapped[chunk].load(Ordering::Relaxed) == MAPPED
    }

    fn protect(&self, start: Address, pages: usize) {
        let start_chunk = Self::address_to_mmap_chunks_down(start);
        let chunks = Self::pages_to_mmap_chunks_up(pages);
        let end_chunk = start_chunk + chunks;
        let guard = self.lock.lock().unwrap();

        for chunk in start_chunk .. end_chunk {
            if self.mapped[chunk].load(Ordering::Relaxed) == MAPPED {
                let mmap_start = Self::mmap_chunks_to_address(chunk);
                if unsafe{mprotect(mmap_start.to_mut_ptr(), MMAP_CHUNK_BYTES,
                                   PROT_NONE) != 0} {
                    drop(guard);
                    panic!("Mmapper.mprotect failed");
                } else {
                    if VERBOSE {
                        trace!("mprotect succeeded at chunk {}  {} with len = {}", chunk,
                               mmap_start, MMAP_CHUNK_BYTES);
                    }
                }
                self.mapped[chunk].store(PROTECTED, Ordering::Relaxed);
            } else {
                debug_assert!(self.mapped[chunk].load(Ordering::Relaxed) == PROTECTED);
            }
        }
        drop(guard);
    }
}

impl ByteMapMmapper {
    pub fn new() -> Self {
        // Hacky because AtomicU8 does not implement Copy
        // Should be fiiine because AtomicXXX has the same bit representation as XXX
        ByteMapMmapper {
            lock: Mutex::new(()),
            mapped: unsafe{transmute([UNMAPPED; MMAP_NUM_CHUNKS])},
        }
    }

    fn bytes_to_mmap_chunks_up(bytes: usize) -> usize {
        (bytes + MMAP_CHUNK_BYTES - 1) >> LOG_MMAP_CHUNK_BYTES
    }

    fn pages_to_mmap_chunks_up(pages: usize) -> usize {
        Self::bytes_to_mmap_chunks_up(pages_to_bytes(pages))
    }

    fn address_to_mmap_chunks_down(addr: Address) -> usize {
        addr >> LOG_MMAP_CHUNK_BYTES
    }

    fn mmap_chunks_to_address(chunk: usize) -> Address {
        unsafe{Address::from_usize(chunk << LOG_MMAP_CHUNK_BYTES)}
    }

    fn address_to_mmap_chunks_up(addr: Address) -> usize {
        (addr + MMAP_CHUNK_BYTES - 1) >> LOG_MMAP_CHUNK_BYTES
    }
}