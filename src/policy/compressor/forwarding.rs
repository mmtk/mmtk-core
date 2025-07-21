use crate::policy::compressor::GC_MARK_BIT_MASK;
use crate::util::constants::BYTES_IN_WORD;
use crate::util::heap::MonotonePageResource;
use crate::util::metadata::side_metadata::spec_defs::{COMPRESSOR_MARK, COMPRESSOR_OFFSET_VECTOR};
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::{Address, ObjectReference};
use crate::vm::object_model::ObjectModel;
use crate::vm::VMBinding;
use atomic::Ordering;
use std::marker::PhantomData;
use std::sync::atomic::AtomicBool;

/// A finite-state machine which visits the positions of marked bits in
/// the mark bitmap, and accumulates the size of live data that it has
/// seen between marked bits.
///
/// The Compressor caches the state of the transducer at the start of
/// each block by serialising the state using [`Transducer::encode`], and
/// then deserialises the state whilst computing forwarding pointers
/// using [`Transducer::decode`].
#[derive(Debug)]
struct Transducer {
    // The total live data visited by the transducer.
    live: usize,
    // The address of the last mark bit which the transducer visited.
    last_bit_visited: Address,
    // Whether or not the transducer is currently inside an object
    // (i.e. if it has seen a first bit but no matching last bit yet).
    in_object: bool,
}
impl Transducer {
    pub fn new() -> Self {
        Self {
            live: 0,
            last_bit_visited: Address::ZERO,
            in_object: false,
        }
    }
    pub fn visit_mark_bit(&mut self, address: Address) {
        if self.in_object {
            // The size of an object is the distance between the end and
            // start of the object, and the last word of the object is one
            // word prior to the end of the object. Thus we must add an
            // extra word, in order to compute the size of the object based
            // on the distance between its first and last words.
            let first_word = self.last_bit_visited;
            let last_word = address;
            let size = last_word - first_word + BYTES_IN_WORD;
            self.live += size;
        }
        self.in_object = !self.in_object;
        self.last_bit_visited = address;
    }

    pub fn encode(&self, current_position: Address) -> usize {
        if self.in_object {
            // We count the space between the last mark bit and
            // the current address as live when we stop in the
            // middle of an object.
            self.live + (current_position - self.last_bit_visited) + 1
        } else {
            self.live
        }
    }

    pub fn decode(offset: usize, current_position: Address) -> Self {
        Transducer {
            live: offset & !1,
            last_bit_visited: current_position,
            in_object: (offset & 1) == 1,
        }
    }
}

pub struct ForwardingMetadata<VM: VMBinding> {
    pub(crate) first_address: Address,
    calculated: AtomicBool,
    vm: PhantomData<VM>,
}

// A block in the Compressor is the granularity at which we record
// the live data prior to the start of each block. We set it to 512 bytes
// following the paper.
pub(crate) const LOG_BLOCK_SIZE: usize = 9;
pub(crate) const BLOCK_SIZE: usize = 1 << LOG_BLOCK_SIZE;
pub(crate) const MARK_SPEC: SideMetadataSpec = COMPRESSOR_MARK;
pub(crate) const OFFSET_VECTOR_SPEC: SideMetadataSpec = COMPRESSOR_OFFSET_VECTOR;

impl<VM: VMBinding> ForwardingMetadata<VM> {
    pub fn new(start: Address) -> ForwardingMetadata<VM> {
        ForwardingMetadata {
            first_address: start,
            calculated: AtomicBool::new(false),
            vm: PhantomData,
        }
    }

    pub fn mark_last_word_of_object(&self, object: ObjectReference) {
        let last_word_of_object = object.to_object_start::<VM>()
            + VM::VMObjectModel::get_current_size(object)
            - BYTES_IN_WORD;
        #[cfg(debug_assertions)]
        {
            // We require to be able to iterate upon first and last bits in the
            // same bitmap. Therefore the first and last bits cannot be the
            // same, else we would only encounter one of the two bits.
            // This requirement implies that objects must be at least two words
            // large.
            debug_assert!(
                MARK_SPEC.are_different_metadata_bits(
                    object.to_object_start::<VM>(),
                    last_word_of_object
                ),
                "The first and last mark bits should be different bits."
            );
        }

        MARK_SPEC.fetch_or_atomic(last_word_of_object, GC_MARK_BIT_MASK, Ordering::SeqCst);
    }

    pub fn calculate_offset_vector(&self, pr: &MonotonePageResource<VM>) {
        let mut state = Transducer::new();
        let last_block = (pr.cursor() - self.first_address) / BLOCK_SIZE;
        debug!("calculating offset of {last_block} blocks");
        for block in 0..last_block {
            let block_start = self.first_address + (block * BLOCK_SIZE);
            let block_end = block_start + BLOCK_SIZE;
            OFFSET_VECTOR_SPEC.store_atomic::<usize>(
                block_start,
                state.encode(block_start),
                Ordering::Relaxed,
            );
            MARK_SPEC.scan_non_zero_values::<u8>(block_start, block_end, &mut |addr: Address| {
                state.visit_mark_bit(addr);
            });
        }
        self.calculated.store(true, Ordering::Relaxed);
    }

    pub fn release(&self) {
        self.calculated.store(false, Ordering::Relaxed);
    }

    pub fn forward(&self, address: Address) -> Address {
        debug_assert!(
            self.calculated.load(Ordering::Relaxed),
            "forward() should only be called when we have calculated an offset vector"
        );
        let block_number = (address - self.first_address) / BLOCK_SIZE;
        let block_address = self.first_address + (block_number * BLOCK_SIZE);
        let mut state = Transducer::decode(
            OFFSET_VECTOR_SPEC.load_atomic::<usize>(block_address, Ordering::Relaxed),
            block_address,
        );
        // The transducer in this implementation computes the offset
        // relative to the start of the heap; whereas Total-Live-Data in
        // the paper computes the offset relative to the start of the block.
        MARK_SPEC.scan_non_zero_values::<u8>(block_address, address, &mut |addr: Address| {
            state.visit_mark_bit(addr)
        });
        self.first_address + state.live
    }

    pub fn scan_marked_objects(
        &self,
        start: Address,
        end: Address,
        f: &mut impl FnMut(ObjectReference),
    ) {
        let mut in_object = false;
        MARK_SPEC.scan_non_zero_values::<u8>(start, end, &mut |addr: Address| {
            if !in_object {
                let object = ObjectReference::from_raw_address(addr).unwrap();
                f(object);
            }
            in_object = !in_object;
        });
    }
}
