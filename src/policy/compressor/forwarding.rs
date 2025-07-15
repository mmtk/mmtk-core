use crate::policy::compressor::GC_MARK_BIT_MASK;
use crate::util::constants::MIN_OBJECT_SIZE;
use crate::util::heap::MonotonePageResource;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::{Address, ObjectReference};
use crate::vm::object_model::ObjectModel;
use crate::vm::VMBinding;
use atomic::Ordering;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicUsize};

/// A finite-state machine which processes the positions of mark bits,
/// and accumulates the size of live data that it has seen.
///
/// The Compressor caches the state of the transducer at the start of
/// each block by serialising the state using [`Transducer::encode`];
/// the state can then be deserialised using [`Transducer::decode`].
#[derive(Debug)]
struct Transducer {
    live: usize,
    last_bit_seen: Address,
    in_object: bool,
}
impl Transducer {
    pub fn new() -> Self {
        Self {
            live: 0,
            last_bit_seen: Address::ZERO,
            in_object: false,
        }
    }
    pub fn step(&mut self, address: Address) {
        if self.in_object {
            self.live += address - self.last_bit_seen + MIN_OBJECT_SIZE;
        }
        self.in_object = !self.in_object;
        self.last_bit_seen = address;
    }

    pub fn encode(&self, address: Address) -> usize {
        if self.in_object {
            // We count the space between the last mark bit and
            // the current address as live when we stop in the
            // middle of an object.
            self.live + (address - self.last_bit_seen) + 1
        } else {
            self.live
        }
    }

    pub fn decode(offset: usize, address: Address) -> Self {
        Transducer {
            live: offset & !1,
            last_bit_seen: address,
            in_object: (offset & 1) == 1,
        }
    }
}

pub struct ForwardingMetadata<VM: VMBinding> {
    mark_bit_spec: SideMetadataSpec,
    pub(crate) first_address: Address,
    calculated: AtomicBool,
    block_offsets: Vec<AtomicUsize>,
    vm: PhantomData<VM>,
}

const BLOCK_SIZE: usize = 512;

impl<VM: VMBinding> ForwardingMetadata<VM> {
    pub fn new(
        mark_bit_spec: SideMetadataSpec,
        start: Address,
        size: usize,
    ) -> ForwardingMetadata<VM> {
        let mut block_offsets = vec![];
        let blocks = size / BLOCK_SIZE;
        block_offsets.resize_with(blocks, || AtomicUsize::new(0));
        ForwardingMetadata {
            mark_bit_spec,
            first_address: start,
            calculated: AtomicBool::new(false),
            block_offsets,
            vm: PhantomData,
        }
    }

    pub fn mark_end_of_object(&self, object: ObjectReference) {
        use crate::util::metadata::side_metadata::{address_to_meta_address, meta_byte_lshift};
        let end_of_object =
            object.to_raw_address() + VM::VMObjectModel::get_current_size(object) - MIN_OBJECT_SIZE;
        let a1 = address_to_meta_address(&self.mark_bit_spec, object.to_raw_address());
        let s1 = meta_byte_lshift(&self.mark_bit_spec, object.to_raw_address());
        let a2 = address_to_meta_address(&self.mark_bit_spec, end_of_object);
        let s2 = meta_byte_lshift(&self.mark_bit_spec, end_of_object);
        debug_assert!((a1, s1) < (a2, s2));

        self.mark_bit_spec
            .fetch_or_atomic(end_of_object, GC_MARK_BIT_MASK, Ordering::SeqCst);
    }

    pub fn calculate_offset_vector(&self, pr: &MonotonePageResource<VM>) {
        let mut state = Transducer::new();
        let last_block = (pr.cursor() - self.first_address) / BLOCK_SIZE;
        debug!("calculating offset of {last_block} blocks");
        for block in 0..last_block {
            let block_start = self.first_address + (block * BLOCK_SIZE);
            let block_end = block_start + BLOCK_SIZE;
            self.block_offsets[block].store(state.encode(block_start), Ordering::Relaxed);
            self.mark_bit_spec.scan_non_zero_values::<u8>(
                block_start,
                block_end,
                &mut |addr: Address| {
                    state.step(addr);
                },
            );
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
            self.block_offsets[block_number].load(Ordering::Relaxed),
            block_address,
        );
        // The transducer in this implementation computes the offset
        // relative to the start of the heap; whereas Total-Live-Data in
        // the paper computes the offset relative to the start of the block.
        self.mark_bit_spec.scan_non_zero_values::<u8>(
            block_address,
            address,
            &mut |addr: Address| {
                state.step(addr);
            },
        );
        self.first_address + state.live
    }

    pub fn scan_marked_objects(
        &self,
        start: Address,
        end: Address,
        f: &mut impl FnMut(ObjectReference),
    ) {
        let mut in_object = false;
        self.mark_bit_spec
            .scan_non_zero_values::<u8>(start, end, &mut |addr: Address| {
                if !in_object {
                    let object = ObjectReference::from_raw_address(addr).unwrap();
                    f(object);
                }
                in_object = !in_object;
            });
    }
}
