use crate::policy::compressor::GC_MARK_BIT_MASK;
use crate::util::constants::MIN_OBJECT_SIZE;
use crate::util::metadata::side_metadata::spec_defs::{COMPRESSOR_MARK, COMPRESSOR_OFFSET_VECTOR};
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::{Address, ObjectReference};
use crate::vm::object_model::ObjectModel;
use crate::vm::VMBinding;
use atomic::Ordering;
use std::marker::PhantomData;
use std::sync::atomic::AtomicBool;

pub(crate) struct ObjectVectorRegion {
    pub from_start: Address,
    pub from_size: usize,
    pub to_start: Address,
}

pub(crate) struct CopyRegion {
    pub from_start: Address,
    pub from_size: usize,
}

/// A finite-state machine which processes the positions of mark bits,
/// and accumulates the start address of an object to be forwarded after
/// all live data that the transducer has seen.
///
/// The Compressor caches the state of the transducer at the start of
/// each block by serialising the state using [`Transducer::encode`];
/// the state can then be deserialised using [`Transducer::decode`].
#[derive(Debug)]
struct Transducer {
    to: Address,
    last_bit_seen: Address,
    in_object: bool,
}
impl Transducer {
    pub fn new(to: Address) -> Self {
        Self {
            to,
            last_bit_seen: Address::ZERO,
            in_object: false,
        }
    }
    pub fn step(&mut self, address: Address) {
        if self.in_object {
            self.to += address - self.last_bit_seen + MIN_OBJECT_SIZE;
        }
        self.in_object = !self.in_object;
        self.last_bit_seen = address;
    }

    pub fn encode(&self, address: Address) -> usize {
        if self.in_object {
            // We count the space between the last mark bit and
            // the current address as live when we stop in the
            // middle of an object.
            self.to.as_usize() + (address - self.last_bit_seen) + 1
        } else {
            self.to.as_usize()
        }
    }

    pub fn decode(value: usize, address: Address) -> Self {
        Transducer {
            to: unsafe { Address::from_usize(value & !1) },
            last_bit_seen: address,
            in_object: (value & 1) == 1,
        }
    }
}

pub struct ForwardingMetadata<VM: VMBinding> {
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
    pub fn new() -> ForwardingMetadata<VM> {
        ForwardingMetadata {
            calculated: AtomicBool::new(false),
            vm: PhantomData,
        }
    }

    pub fn mark_end_of_object(&self, object: ObjectReference) {
        let end_of_object = object.to_object_start::<VM>()
            + VM::VMObjectModel::get_current_size(object)
            - MIN_OBJECT_SIZE;
        #[cfg(debug_assertions)]
        {
            use crate::util::metadata::side_metadata::{address_to_meta_address, meta_byte_lshift};
            // We require to be able to iterate upon start and end bits in the
            // same bitmap. Therefore the start and end bits cannot be the
            // same, else we would only encounter one of the two bits.
            let a1 = address_to_meta_address(&MARK_SPEC, object.to_raw_address());
            let s1 = meta_byte_lshift(&MARK_SPEC, object.to_raw_address());
            let a2 = address_to_meta_address(&MARK_SPEC, end_of_object);
            let s2 = meta_byte_lshift(&MARK_SPEC, end_of_object);
            debug_assert!(
                (a1, s1) < (a2, s2),
                "The start and end mark bits should be different bits"
            );
        }

        MARK_SPEC.fetch_or_atomic(end_of_object, GC_MARK_BIT_MASK, Ordering::SeqCst);
    }

    pub fn calculate_offset_vector(&self, region: &ObjectVectorRegion) {
        let mut state = Transducer::new(region.to_start);
        let last_block = region.from_size / BLOCK_SIZE;
        debug!("calculating offset of {last_block} blocks");
        for block in 0..last_block {
            let block_start = region.from_start + (block * BLOCK_SIZE);
            let block_end = block_start + BLOCK_SIZE;
            OFFSET_VECTOR_SPEC.store_atomic::<usize>(
                block_start,
                state.encode(block_start),
                Ordering::Relaxed,
            );
            MARK_SPEC.scan_non_zero_values::<u8>(block_start, block_end, &mut |addr: Address| {
                state.step(addr);
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
        // Round down the address to its block.
        let block_address = unsafe {
            Address::from_usize(address.as_usize() & !(BLOCK_SIZE - 1))
        };
        let mut state = Transducer::decode(
            OFFSET_VECTOR_SPEC.load_atomic::<usize>(block_address, Ordering::Relaxed),
            block_address,
        );
        // The transducer in this implementation computes the final
        // address of an object; whereas Total-Live-Data in the paper computes
        // the distance of the object from the start of the block.
        MARK_SPEC.scan_non_zero_values::<u8>(block_address, address, &mut |addr: Address| {
            state.step(addr);
        });
        state.to
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
