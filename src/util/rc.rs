use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, AtomicUsize};

use crate::util::heap::chunk_map::Chunk;
use crate::util::linear_scan::Region;
use crate::util::{metadata::side_metadata::address_to_meta_address, Address};
use crate::{
    policy::immix::{block::Block, line::Line},
    util::{metadata::side_metadata::SideMetadataSpec, ObjectReference},
    vm::*,
};
use atomic::Ordering;

/// Log2 of the number of bits used to store each object's reference count in the RC table.
pub const LOG_REF_COUNT_BITS: usize = 1;
/// Number of bits used to store each object's reference count in the RC table.
pub const REF_COUNT_BITS: u8 = 1 << LOG_REF_COUNT_BITS;
/// Bit mask covering the bits used to store a reference count.
pub const REF_COUNT_MASK: u8 = (((1u16 << REF_COUNT_BITS) - 1) & 0xff) as u8;
/// The maximum representable reference count. Once an object's count reaches this value it
/// is treated as saturated/sticky and is no longer incremented or decremented.
pub const MAX_REF_COUNT: u8 = REF_COUNT_MASK;

/// Log2 of the minimum object size, i.e. the granularity at which reference counts are tracked.
pub const LOG_MIN_OBJECT_SIZE: usize = crate::util::constants::LOG_MIN_OBJECT_SIZE as _;
/// The minimum object size, i.e. the granularity at which reference counts are tracked.
pub const MIN_OBJECT_SIZE: usize = 1 << LOG_MIN_OBJECT_SIZE;

/// Side metadata recording which Immix lines are "straddled" by an object that spans
/// multiple lines, so straddling objects can be identified without scanning their contents.
pub const RC_STRADDLE_LINES: SideMetadataSpec =
    crate::util::metadata::side_metadata::spec_defs::RC_STRADDLE_LINES;

/// Side metadata spec for the per-object reference count table.
pub const RC_TABLE: SideMetadataSpec = crate::util::metadata::side_metadata::spec_defs::RC_TABLE;

/// The start of an object's *envelope*: the address where LXR stores the object's reference count and
/// straddle marks on, in place of either the object reference or the real object start.
///
/// The envelope is guaranteed to cover the entire memory the object occupies. But unlike computing the object
/// start (which may be at a variable offset from the object reference), the envelope start can be computed
/// from object reference with constants only. This makes it more efficient to access.
///
/// ```text
///                            |<--- UPPER --->|
///                            |<-slack->|
///                            |         |<-L->|
///                            |         |     |                      |
/// envelope                   [######################################]
/// object, offset = UPPER     [===============o============]
/// object, offset = LOWER               [=====o======================]
///                            ^         ^     ^
///                            |         |     ref  (L = LOWER; the only address we know)
///                            |         latest possible object start = ref - LOWER
///                            earliest possible object start = ref - UPPER = envelope start
/// ```
///
/// All the RC metadata is stored at the envelope start.
///
/// To avoid the envelope start being outside of the block where the object is in, we reserve the leading 'slack'
/// bytes of each block for the envelope reservation. See `ImmixAllocator::block_usable_start`.
///
/// TODO: Currently both LOS and Immix objects in LXR are using object envelope. But we don't reserve 'slack' for
/// large objects space.
#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectEnvelope(Address);

impl ObjectEnvelope {
    /// The width of the bracket the envelope puts around an object, `UPPER - LOWER`: how far its
    /// start can precede the object's real start.
    pub const fn slack<VM: VMBinding>() -> usize {
        (VM::VMObjectModel::OBJECT_REF_OFFSET_UPPER_BOUND
            - VM::VMObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND) as usize
    }

    /// The constant distance between an object's reference address and its envelope start.
    const fn offset<VM: VMBinding>() -> usize {
        VM::VMObjectModel::OBJECT_REF_OFFSET_UPPER_BOUND as usize
    }

    /// The envelope's exclusive end for an object of `size` bytes: the latest address the object
    /// could possibly end at, i.e. `ref - LOWER + size`.
    pub fn end<VM: VMBinding>(self, size: usize) -> Address {
        self.0 + size + Self::slack::<VM>()
    }

    /// Whether an object of `size` bytes needs straddle marks, i.e. whether its envelope can
    /// touch a line other than the one holding its reference count and the one after it.
    ///
    /// The test is on the *envelope's* length, `size + slack`, not the object's: a range longer
    /// than a line can touch three or more lines, and every line but the first and last needs a
    /// mark. That is why the threshold is `Line::BYTES - slack` rather than `Line::BYTES` --
    /// with a coarse bound this pulls a lot more objects into straddle marking, which is the
    /// price of deriving the envelope from a single global offset.
    ///
    /// Every guard that decides whether to mark, unmark, or assert goes through here, so they
    /// cannot disagree: a mark that unmark does not clear is a stray count that outlives its
    /// object.
    pub fn needs_straddle_marks<VM: VMBinding>(size: usize) -> bool {
        size + Self::slack::<VM>() > Line::BYTES
    }

    /// The envelope of object `o`: a plain constant subtraction.
    pub fn of<VM: VMBinding>(o: ObjectReference) -> Self {
        debug_assert!(
            VM::VMObjectModel::OBJECT_REF_OFFSET_UPPER_BOUND
                >= VM::VMObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND,
            "OBJECT_REF_OFFSET_UPPER_BOUND must not be below the lower bound"
        );
        let a = o.to_raw_address();
        let envelope = a - Self::offset::<VM>();
        // The reservations above are what keep this true. If it ever fires, either an allocation
        // path is handing out the leading `slack` bytes of a region, or a count is being read for
        // an object in a space that reserves nothing because it is not reference counted.
        debug_assert!(
            envelope >= Chunk::from_unaligned_address(a).start(),
            "envelope start {} for {} escaped its chunk; the region it was allocated in did not \
            reserve ObjectEnvelope::slack, or {} is not in a reference-counted space",
            envelope,
            o,
            o,
        );
        Self(envelope)
    }

    /// The envelope whose start is `a`. `a` must be an envelope start, i.e. an address that
    /// [`Self::of`] produced; the reference-count table only ever holds counts at such addresses.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that `a` is an envelope start, so that
    /// [`Self::to_object_reference`] reproduces a real object reference.
    pub unsafe fn from_start(a: Address) -> Self {
        Self(a)
    }

    /// The object this envelope belongs to. Exactly inverts [`Self::of`].
    pub fn to_object_reference<VM: VMBinding>(self) -> ObjectReference {
        // Safety: an envelope start is derived from a non-zero, word-aligned object reference by
        // subtracting a constant, so adding it back reproduces that reference.
        unsafe { ObjectReference::from_raw_address_unchecked(self.0 + Self::offset::<VM>()) }
    }

    /// The envelope's start address, i.e. the address its reference count is stored at.
    pub fn start(self) -> Address {
        self.0
    }

    /// The line holding the envelope's start.
    pub fn line(self) -> Line {
        Line::from_unaligned_address(self.0)
    }
}

static INC_BUFFER_SIZE: AtomicUsize = AtomicUsize::new(0);

static TOTAL_INCS_PACKETS: AtomicU32 = AtomicU32::new(0);

static TOTAL_INCS: AtomicU32 = AtomicU32::new(0);
static ROOT_INCS: AtomicU32 = AtomicU32::new(0);
static MATURE_INCS: AtomicU32 = AtomicU32::new(0);
static NURSERY_INCS: AtomicU32 = AtomicU32::new(0);
static FAST_NURSERY_INCS: AtomicU32 = AtomicU32::new(0);
static LOS_INCS: AtomicU32 = AtomicU32::new(0);

static PROMOTED_OBJECTS: AtomicU32 = AtomicU32::new(0);
static PROMOTED_SCALARS: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
static PROMOTED_PRIM_ARRAYS: [AtomicU32; 3] =
    [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
static PROMOTED_OBJECT_ARRAYS: [AtomicU32; 3] =
    [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];

/// A zero-sized helper type providing methods to read and update per-object reference count
/// metadata for LXR's reference counting plan.
#[repr(transparent)]
#[derive(Debug, Copy)]
pub struct RefCountHelper<VM: VMBinding>(PhantomData<VM>);

impl<VM: VMBinding> RefCountHelper<VM> {
    /// A singleton instance of `RefCountHelper` (the type is zero-sized, so it can be freely copied/cloned).
    pub const NEW: Self = Self(PhantomData);
    /// Whether extra reference-counting sanity checks are enabled (debug builds or the `sanity` feature).
    pub const SANITY: bool = cfg!(debug_assertions) || cfg!(feature = "sanity");

    /// Returns the current size of the global increment buffer, i.e. the number of pending
    /// reference count increments that have been enqueued but not yet processed.
    pub fn inc_buffer_size(&self) -> usize {
        INC_BUFFER_SIZE.load(Ordering::Relaxed)
    }

    /// Increases the global increment buffer size counter by `delta`.
    pub fn increase_inc_buffer_size(&self, delta: usize) {
        INC_BUFFER_SIZE.store(
            INC_BUFFER_SIZE
                .load(Ordering::Relaxed)
                .saturating_add(delta),
            Ordering::Relaxed,
        );
    }

    /// Resets the global increment buffer size counter to zero.
    pub fn reset_inc_buffer_size(&self) {
        INC_BUFFER_SIZE.store(0, Ordering::Relaxed)
    }

    /// The address object `o`'s reference count is stored at: its envelope start
    fn rc_slot(&self, o: ObjectReference) -> Address {
        ObjectEnvelope::of::<VM>(o).start()
    }

    /// Atomically updates the reference count of object `o` by applying `f` to its current
    /// value, following the same semantics as `AtomicU8::fetch_update`.
    pub fn fetch_update(
        &self,
        o: ObjectReference,
        f: impl FnMut(u8) -> Option<u8>,
    ) -> Result<u8, u8> {
        RC_TABLE.fetch_update_atomic(self.rc_slot(o), Ordering::Relaxed, Ordering::Relaxed, f)
    }

    /// Returns `true` if object `o`'s reference count has saturated at `MAX_REF_COUNT` (sticky).
    pub fn is_stuck(&self, o: ObjectReference) -> bool {
        self.count(o) == MAX_REF_COUNT
    }

    /// Forces object `o`'s reference count to `MAX_REF_COUNT`, permanently marking it as sticky
    /// so it is never reclaimed by reference counting.
    pub fn stick(&self, o: ObjectReference) -> Result<u8, u8> {
        self.fetch_update(o, |x| {
            debug_assert!(x <= MAX_REF_COUNT);
            if x == MAX_REF_COUNT {
                None
            } else {
                Some(MAX_REF_COUNT)
            }
        })
    }

    /// Increments object `o`'s reference count by one, leaving it unchanged (saturating) once
    /// it has reached `MAX_REF_COUNT`.
    pub fn inc(&self, o: ObjectReference) -> Result<u8, u8> {
        #[cfg(feature = "vo_bit")]
        debug_assert!(
            crate::util::metadata::vo_bit::is_vo_bit_set(o),
            "{o}: VO bit not set",
        );

        self.fetch_update(o, |x| {
            debug_assert!(x <= MAX_REF_COUNT);
            if x == MAX_REF_COUNT {
                None
            } else {
                Some(x + 1)
            }
        })
    }

    /// Decrements object `o`'s reference count by one, unless it is already zero or has
    /// saturated at `MAX_REF_COUNT` (sticky), in which case it is left unchanged.
    pub fn dec(&self, o: ObjectReference) -> Result<u8, u8> {
        #[cfg(feature = "vo_bit")]
        debug_assert!(
            crate::util::metadata::vo_bit::is_vo_bit_set(o),
            "{o}: VO bit not set",
        );

        self.fetch_update(o, |x| {
            debug_assert!(x <= MAX_REF_COUNT);
            if x == 0 || x == MAX_REF_COUNT
            /* sticky */
            {
                None
            } else {
                Some(x - 1)
            }
        })
    }

    /// Atomically sets object `o`'s reference count to `count`.
    pub fn set(&self, o: ObjectReference, count: u8) {
        RC_TABLE.store_atomic(self.rc_slot(o), count, Ordering::Relaxed)
    }

    /// Sets object `o`'s reference count to `count` using a non-atomic store, for use where the
    /// caller can guarantee there is no concurrent access.
    pub fn set_relaxed(&self, o: ObjectReference, count: u8) {
        unsafe { RC_TABLE.store(self.rc_slot(o), count) }
    }

    /// Sets the reference count for the line containing object `o` to `count` using a non-atomic store,
    /// for use where the caller can guarantee there is no concurrent access.
    pub fn set_line_relaxed(&self, line: Line, count: u8) {
        unsafe { RC_TABLE.store(line.start(), count) }
    }

    /// Returns object `o`'s current reference count.
    pub fn count(&self, o: ObjectReference) -> u8 {
        RC_TABLE.load_atomic(self.rc_slot(o), Ordering::Relaxed)
    }

    /// Returns the reference count stored in the RC table at address `addr`. If this
    /// returns a non-zero value, it indicates that `addr` is the address of an object reference,
    /// or the start of a straddle line.
    pub fn count_by_address(&self, addr: Address) -> u8 {
        RC_TABLE.load_atomic(addr, Ordering::Relaxed)
    }

    /// Returns `true` if the RC table entry at `o`'s address is zero. Used for both individual
    /// objects and line-granularity entries (e.g. straddle line markers), which share the same table.
    pub fn object_or_line_is_dead(&self, o: ObjectReference) -> bool {
        RC_TABLE.load_byte(self.rc_slot(o)) == 0
    }

    /// Returns a slice view over the raw RC table memory covering block `b`, reinterpreted as an
    /// array of `UInt`, allowing the block's reference counts to be scanned in bulk.
    pub fn rc_table_range<UInt: Sized>(&self, b: Block) -> &'static [UInt] {
        debug_assert!({
            let log_bits_in_uint: usize =
                (std::mem::size_of::<UInt>() << 3).trailing_zeros() as usize;
            Block::LOG_BYTES - super::rc::LOG_MIN_OBJECT_SIZE + super::rc::LOG_REF_COUNT_BITS
                >= log_bits_in_uint
        });
        let start = address_to_meta_address(&super::rc::RC_TABLE, b.start()).to_ptr::<UInt>();
        let limit = address_to_meta_address(&super::rc::RC_TABLE, b.end()).to_ptr::<UInt>();
        let rc_table = unsafe { std::slice::from_raw_parts(start, limit.offset_from(start) as _) };
        rc_table
    }

    /// Returns `true` if object `o`'s reference count is zero.
    #[allow(unused)]
    pub fn is_dead(&self, o: ObjectReference) -> bool {
        let v: u8 = RC_TABLE.load_atomic(self.rc_slot(o), Ordering::Relaxed);
        v == 0
    }

    /// Returns `true` if object `o`'s reference count is zero (dead) or has saturated at
    /// `MAX_REF_COUNT` (sticky).
    pub fn is_dead_or_stuck(&self, o: ObjectReference) -> bool {
        let v: u8 = RC_TABLE.load_atomic(self.rc_slot(o), Ordering::Relaxed);
        v == 0 || v == MAX_REF_COUNT
    }

    /// Returns `true` if object `o` is in a straddle line. The function does not check rc table.
    pub fn object_is_in_straddle_line_no_rc_check(&self, o: ObjectReference) -> bool {
        // This directly reads line-granularity straddle line metadata with an unaligned address.
        // It is still correct, but may break side metadata assertions.
        unsafe { RC_STRADDLE_LINES.load::<u8>(self.rc_slot(o)) != 0 }
    }

    /// Returns `true` if address `a` falls within a live object whose containing line is marked
    /// as a straddle line.
    pub fn object_is_in_straddle_line(&self, o: ObjectReference) -> bool {
        let line = ObjectEnvelope::of::<VM>(o).line();
        self.count(o) != 0 && unsafe { RC_STRADDLE_LINES.load::<u8>(line.start()) != 0 }
    }

    /// The `[start, end)` range of lines that need a synthetic straddle mark for object `o`.
    fn straddle_line_range(&self, o: ObjectReference, size: usize) -> (Line, Line) {
        let envelope = ObjectEnvelope::of::<VM>(o);
        let start_line = envelope.line().next();
        let end_line = Line::from_unaligned_address(envelope.end::<VM>(size));
        let block_end_line = Block::from_unaligned_address(o.to_raw_address()).end_line();
        let end_line = if end_line > block_end_line {
            block_end_line
        } else {
            end_line
        };
        // A clamp can put the end at or before the start when the object sits at the very end of
        // its block; an empty range is correct there, so normalise rather than wrap around.
        if end_line < start_line {
            (start_line, start_line)
        } else {
            (start_line, end_line)
        }
    }

    fn mark_straddle_object_with_size(&self, o: ObjectReference, size: usize) {
        debug_assert!(ObjectEnvelope::needs_straddle_marks::<VM>(size));
        let (start_line, end_line) = self.straddle_line_range(o, size);
        let mut line = start_line;
        while line != end_line {
            unsafe { RC_STRADDLE_LINES.store(line.start(), 1u8) };
            self.set_line_relaxed(line, 1);
            line = line.next();
        }
    }

    /// Marks every line (other than the first) spanned by object `o` as a straddle line, so the
    /// object can be identified from any of the lines it straddles.
    pub fn mark_straddle_object(&self, o: ObjectReference) {
        let size = VM::VMObjectModel::get_current_size(o);
        self.mark_straddle_object_with_size(o, size)
    }

    /// Clears the straddle-line and reference-count markers set by `mark_straddle_object` for
    /// every line (other than the first) spanned by object `o`.
    pub fn unmark_straddle_object(&self, o: ObjectReference) {
        // debug_assert!(crate::args::RC_NURSERY_EVACUATION);
        let size = VM::VMObjectModel::get_current_size(o);
        if ObjectEnvelope::needs_straddle_marks::<VM>(size) {
            let (start_line, end_line) = self.straddle_line_range(o, size);
            let mut line = start_line;
            while line != end_line {
                self.set_line_relaxed(line, 0);
                unsafe { RC_STRADDLE_LINES.store(line.start(), 0u8) };
                line = line.next();
            }
        }
    }

    /// Debug assertion that every `MIN_OBJECT_SIZE` granule within object `o` has a reference
    /// count of zero, used to verify that a reclaimed object has been fully cleared.
    pub fn assert_zero_ref_count(&self, o: ObjectReference) {
        let size = VM::VMObjectModel::get_current_size(o);
        let start = ObjectEnvelope::of::<VM>(o).start();
        for i in (0..size).step_by(MIN_OBJECT_SIZE) {
            let a = start + i;
            assert_eq!(0, self.count_by_address(a));
        }
    }

    /// Called when object `o` is promoted to mature space; marks it as a straddle object if it
    /// spans more than one line, deriving its size from the VM binding.
    pub fn promote(&self, o: ObjectReference) {
        let size = o.get_size::<VM>();
        if ObjectEnvelope::needs_straddle_marks::<VM>(size) {
            self.mark_straddle_object_with_size(o, size);
        }
    }

    /// Same as `promote`, but with the object's size supplied by the caller instead of being
    /// queried from the VM binding.
    pub fn promote_with_size(&self, o: ObjectReference, size: usize) {
        debug_assert!(self.envelope_is_well_formed(o, size));
        if ObjectEnvelope::needs_straddle_marks::<VM>(size) {
            self.mark_straddle_object_with_size(o, size);
        }
    }

    /// The two properties envelope keying relies on, checked per object in debug builds. Only
    /// meaningful for an object in an Immix space, which is the only caller.
    #[cfg(debug_assertions)]
    fn envelope_is_well_formed(&self, o: ObjectReference, size: usize) -> bool {
        let envelope = ObjectEnvelope::of::<VM>(o).start();
        let start = o.to_object_start::<VM>();
        // 1. The envelope covers the object's head, and does not run out of the object's own
        //    block. Escaping the block would put the count in a neighbour's reference-count
        //    table, where `Block::clear_rc_table` would bzero it out from under a live object.
        //    `ImmixAllocator` leaves `ObjectEnvelope::slack` bytes free at each block start so
        //    this holds; if it fails, some allocation path is missing that skip.
        let head_ok = envelope <= start
            && envelope >= Block::from_unaligned_address(o.to_raw_address()).start();
        // 2. The envelope reaches at or past the object's true end, so marking covers every
        //    line the object occupies except at most the last -- and the hole finder's
        //    skip-the-first-line-of-a-hole margin covers exactly one. This is what the envelope
        //    being the *union* of the object's possible positions buys: with a coarse offset
        //    bound the shortfall of a slid-down envelope would span whole lines, leaving two or
        //    more of them uncounted, which one margin cannot absorb.
        let tail_ok = start + size <= ObjectEnvelope::of::<VM>(o).end::<VM>(size);
        head_ok && tail_ok
    }
}

impl<VM: VMBinding> Clone for RefCountHelper<VM> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}
