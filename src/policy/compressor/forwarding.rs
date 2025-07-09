use crate::policy::compressor::GC_MARK_BIT_MASK;
use crate::util::constants::MIN_OBJECT_SIZE;
use crate::vm::VMBinding;
use crate::vm::object_model::ObjectModel;
use crate::util::{Address, ObjectReference};
use crate::util::heap::MonotonePageResource;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::marker::PhantomData;
use atomic::Ordering;

/// A finite-state machine which processes the positions of mark bits,
/// and accumulates the size of live data that it has seen.
#[derive(Debug)]
struct Transducer {
    live: usize,
    last_bit_seen: Address,
    in_object: bool
}
impl Transducer {
    pub fn new() -> Self {
        Self {
            live: 0,
            last_bit_seen: Address::ZERO,
            in_object: false
        }
    }
    pub fn step(&mut self, address: Address) {
        //println!("Address: {self:?} {address:?}");
        if self.in_object {
            self.live += address - self.last_bit_seen + (MIN_OBJECT_SIZE as usize);
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
            in_object: (offset & 1) == 1
        }
    }
}

pub struct ForwardingMetadata<VM: VMBinding> {
    mark_bit_spec: SideMetadataSpec,
    pub(crate) first_address: Address,
    calculated: AtomicBool,
    block_offsets: Vec<AtomicUsize>,
    vm: PhantomData<VM>
}

const BLOCK_SIZE: usize = 512;

impl<VM: VMBinding> ForwardingMetadata<VM> {
    pub fn new(mark_bit_spec: SideMetadataSpec, start: Address, size: usize) -> ForwardingMetadata<VM> {
        let mut block_offsets = vec![];
        let blocks = size / BLOCK_SIZE;
        block_offsets.resize_with(blocks, || AtomicUsize::new(0));
        ForwardingMetadata {
            mark_bit_spec: mark_bit_spec,
            first_address: start,
            calculated: AtomicBool::new(false),
            block_offsets,
            vm: PhantomData
        }
    }
    
    pub fn mark_end_of_object(&self, object: ObjectReference) {
        use crate::util::metadata::side_metadata::{address_to_meta_address, meta_byte_lshift};
        let end_of_object = object.to_raw_address() + VM::VMObjectModel::get_current_size(object) - (MIN_OBJECT_SIZE as usize);
        let a1 = address_to_meta_address(&self.mark_bit_spec, object.to_raw_address());
        let s1 = meta_byte_lshift(&self.mark_bit_spec, object.to_raw_address());
        let a2 = address_to_meta_address(&self.mark_bit_spec, end_of_object);
        let s2 = meta_byte_lshift(&self.mark_bit_spec, end_of_object);
        assert!((a1, s1) < (a2, s2));
        
        self.mark_bit_spec.fetch_or_atomic(
            end_of_object,
            GC_MARK_BIT_MASK,
            Ordering::SeqCst
        );
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
                &mut |addr: Address| { state.step(addr); }
            );
        }
        self.calculated.store(true, Ordering::Relaxed);
    }

    pub fn release(&self) {
        self.calculated.store(false, Ordering::Relaxed);
    }
    
    pub fn forward(&self, address: Address) -> Address {
        debug_assert!(self.calculated.load(Ordering::Relaxed), "forward() should only be called when we have calculated an offset vector");
        let block = (address - self.first_address) / BLOCK_SIZE;
        let block_start = self.first_address + (block * BLOCK_SIZE);
        let mut state = Transducer::decode(self.block_offsets[block].load(Ordering::Relaxed), block_start);
        //println!("running {state:?} from {block_start} to {address}:");
        self.mark_bit_spec.scan_non_zero_values::<u8>(
            block_start,
            address,
            &mut |addr: Address| { state.step(addr); }
        );
        return self.first_address + state.live;
    }
    
    pub fn scan_marked_objects(&self, start: Address, end: Address, f: &mut impl FnMut(ObjectReference)) {
        let mut in_object = false;
        self.mark_bit_spec.scan_non_zero_values::<u8>(
            start,
            end,
            &mut |addr: Address| {
                if !in_object {
                    let object = ObjectReference::from_raw_address(addr).unwrap();
                    f(object);
                }
                in_object = !in_object;
            }
        );
    }
}
