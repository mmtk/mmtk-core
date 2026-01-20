use crate::util::constants::{BYTES_IN_WORD, LOG_BITS_IN_WORD};
use crate::util::linear_scan::{Region, RegionIterator};
use crate::util::metadata::side_metadata::spec_defs::{COMPRESSOR_MARK, COMPRESSOR_OFFSET_VECTOR};
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::options::Options;
use crate::util::{Address, ObjectReference};
use crate::vm::object_model::ObjectModel;
use crate::vm::VMBinding;
use atomic::Ordering;
use std::marker::PhantomData;
use std::sync::atomic::AtomicBool;

/// A [`CompressorRegion`] is the granularity at which [`super::CompressorSpace`]
/// compacts the heap. Objects are allocated inside one region, and are only ever
/// moved *within* that region.
#[derive(Copy, Clone, PartialEq, PartialOrd)]
pub(crate) struct CompressorRegion(Address);
impl Region for CompressorRegion {
    const LOG_BYTES: usize = 18; // 256 kiB
    fn from_aligned_address(address: Address) -> Self {
        assert!(
            address.is_aligned_to(Self::BYTES),
            "{address} is not aligned"
        );
        CompressorRegion(address)
    }
    fn start(&self) -> Address {
        self.0
    }
}

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
    /// The address for the next object to be copied to, following preceding
    /// objects which were visited by the transducer.
    to: Address,
    /// The address of the last mark bit which the transducer visited.
    last_bit_visited: Address,
    /// Whether or not the transducer is currently inside an object
    /// (i.e. if it has seen a first bit but no matching last bit yet).
    in_object: bool,
}
impl Transducer {
    pub fn new(to: Address) -> Self {
        Self {
            to,
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
            self.to += size;
        }
        self.in_object = !self.in_object;
        self.last_bit_visited = address;
    }

    pub fn encode(&self, current_position: Address) -> usize {
        if self.in_object {
            // We count the space between the last mark bit and
            // the current address as live when we stop in the
            // middle of an object.
            self.to.as_usize() + (current_position - self.last_bit_visited) + 1
        } else {
            self.to.as_usize()
        }
    }

    pub fn decode(offset: usize, current_position: Address) -> Self {
        Transducer {
            to: unsafe { Address::from_usize(offset & !1) },
            last_bit_visited: current_position,
            in_object: (offset & 1) == 1,
        }
    }
}

pub struct ForwardingMetadata<VM: VMBinding> {
    calculated: AtomicBool,
    vm: PhantomData<VM>,
    // This field is only used on x86_64.
    _use_clmul: bool,
}

// A block in the Compressor is the granularity at which we cache
// the amount of live data preceding an address. We set it to 512 bytes
// following the paper.
#[derive(Copy, Clone, PartialEq, PartialOrd)]
pub(crate) struct Block(Address);
impl Region for Block {
    const LOG_BYTES: usize = 9; // 512 B
    fn from_aligned_address(address: Address) -> Self {
        assert!(address.is_aligned_to(Self::BYTES));
        Block(address)
    }
    fn start(&self) -> Address {
        self.0
    }
}

pub(crate) const MARK_SPEC: SideMetadataSpec = COMPRESSOR_MARK;
pub(crate) const OFFSET_VECTOR_SPEC: SideMetadataSpec = COMPRESSOR_OFFSET_VECTOR;

impl<VM: VMBinding> ForwardingMetadata<VM> {
    pub fn new(options: &Options) -> ForwardingMetadata<VM> {
        ForwardingMetadata {
            calculated: AtomicBool::new(false),
            vm: PhantomData,
            _use_clmul: *options.compressor_use_clmul,
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

        // We only mark the last word as input to computing forwarding
        // information, so relaxed consistency is okay.
        MARK_SPEC.fetch_or_atomic::<u8>(last_word_of_object, 1, Ordering::Relaxed);
    }

    // TODO: We could compute a prefix-sum by Hillis-Steele too, for which
    // the same offset-vector algorithm works. Would it be faster than the
    // branchy version?

    // SAFETY: Only call this function when the processor supports
    // pclmulqdq and popcnt, i.e. when processor_can_clmul().
    #[cfg(target_arch = "x86_64")]
    unsafe fn calculate_offset_vector_clmul(&self, region: CompressorRegion, cursor: Address) {
        // This function implements Geoff Langdale's
        // algorithm to find quote pairs using prefix sums:
        // https://branchfree.org/2019/03/06/code-fragment-finding-quote-pairs-with-carry-less-multiply-pclmulqdq/

        // We require that each block has at least one word of
        // mark bitmap for this algorithm to work.
        const_assert!(Block::LOG_BYTES - MARK_SPEC.log_bytes_in_region >= LOG_BITS_IN_WORD);
        debug_assert!(processor_can_clmul());
        // We need a local function to use #[target_feature], which in turn
        // allows rustc to generate the POPCNT and PCLMULQDQ instructions.
        #[target_feature(enable = "pclmulqdq,popcnt")]
        unsafe fn inner(to: &mut Address, carry: &mut i64, word: usize, addr: Address) {
            use std::arch::x86_64;
            if addr.is_aligned_to(Block::BYTES) {
                // Write the state at the start of the block.
                // The carry has all bits set the same way,
                // so extract the least significant bit.
                let in_object = (*carry as usize) & 1;
                let encoded = (*to).as_usize() + in_object;
                OFFSET_VECTOR_SPEC.store_atomic::<usize>(addr, encoded, Ordering::Relaxed);
            }
            // Compute the prefix sum of this word of mark bitmap.
            let ones = x86_64::_mm_set1_epi8(0xFFu8 as i8);
            let vector = x86_64::_mm_set_epi64x(0, word as i64);
            let sum: i64 = x86_64::_mm_cvtsi128_si64(x86_64::_mm_clmulepi64_si128(vector, ones, 0));
            debug_assert_eq!(sum, prefix_sum(word) as i64);
            // Carry-in from the last word. If the last word ended in the
            // middle of an object, we need to invert the in/out-of-object
            // states in this word.
            let flipped = sum ^ *carry;
            // Produce a carry-out for the next word. This shift will
            // replicate the most significant bit to all bit positions.
            *carry = flipped >> 63;
            // Now count the in-object bits. The marked bits on either
            // end of an object are both in an object, despite that the
            // prefix sum for the bit at the end of an object will be zero,
            // so we bitwise-or the original word with the prefix sum to
            // find all in-object bits.
            *to += (((flipped as usize | word).count_ones()) * 8) as usize;
        }

        let mut to = region.start();
        let mut carry: i64 = 0;
        MARK_SPEC.scan_words(
            region.start(),
            cursor.align_up(Block::BYTES),
            &mut |word, _, start, end| {
                panic!("should be word aligned, got {word}[{start}:{end}] instead")
            },
            &mut |word: usize, addr: Address| {
                inner(&mut to, &mut carry, word, addr);
            },
        );
    }

    fn calculate_offset_vector_base(&self, region: CompressorRegion, cursor: Address) {
        let mut state = Transducer::new(region.start());
        let first_block = Block::from_aligned_address(region.start());
        let last_block = Block::from_aligned_address(cursor);
        for block in RegionIterator::<Block>::new(first_block, last_block) {
            OFFSET_VECTOR_SPEC.store_atomic::<usize>(
                block.start(),
                state.encode(block.start()),
                Ordering::Relaxed,
            );
            MARK_SPEC.scan_non_zero_values::<u8>(
                block.start(),
                block.end(),
                &mut |addr: Address| {
                    state.visit_mark_bit(addr);
                },
            );
        }
    }

    pub fn calculate_offset_vector(&self, region: CompressorRegion, cursor: Address) {
        #[cfg(target_arch = "x86_64")]
        {
            if self._use_clmul && processor_can_clmul() {
                unsafe {
                    // SAFETY: We checked the processor supports the
                    // necessary instructions.
                    self.calculate_offset_vector_clmul(region, cursor)
                }
            } else {
                self.calculate_offset_vector_base(region, cursor)
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        self.calculate_offset_vector_base(region, cursor);
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
        let block = Block::from_unaligned_address(address);
        let mut state = Transducer::decode(
            OFFSET_VECTOR_SPEC.load_atomic::<usize>(block.start(), Ordering::Relaxed),
            block.start(),
        );
        // The transducer in this implementation computes the final
        // address of an object; whereas Total-Live-Data in the paper computes
        // the distance of the object from the start of the block.
        MARK_SPEC.scan_non_zero_values::<u8>(block.start(), address, &mut |addr: Address| {
            state.visit_mark_bit(addr)
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

    pub fn has_calculated_forwarding_addresses(&self) -> bool {
        self.calculated.load(Ordering::Relaxed)
    }
}

#[cfg(target_arch = "x86_64")]
fn processor_can_clmul() -> bool {
    is_x86_feature_detected!("pclmulqdq") && is_x86_feature_detected!("popcnt")
}

// This function is only used in a debug assertion for the x86_64-only
// calculate_offset_vector_clmul.
#[cfg(target_arch = "x86_64")]
fn prefix_sum(x: usize) -> usize {
    // This function implements a bit-parallel version of the Hillis-Steele prefix sum algorithm:
    // https://en.wikipedia.org/wiki/Prefix_sum#Algorithm_1:_Shorter_span,_more_parallel
    let mut result = x;
    let mut n = 1;
    while n < usize::BITS {
        result ^= result << n;
        n <<= 1;
    }
    result
}
