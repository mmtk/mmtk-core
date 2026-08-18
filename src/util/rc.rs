use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, AtomicUsize};

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
    ///
    /// A `fetch_add` rather than a load-then-store: every mutator's barrier flush lands here, so a
    /// read-modify-write that is not atomic loses almost every update under contention. Measured on
    /// `tree_mutable`, the counter read back 0-10918 for pauses whose `ProcessIncs` packets handled
    /// millions of increments, which made any trigger built on `inc_buffer_size` inert.
    pub fn increase_inc_buffer_size(&self, delta: usize) {
        INC_BUFFER_SIZE.fetch_add(delta, Ordering::Relaxed);
    }

    /// Resets the global increment buffer size counter to zero.
    pub fn reset_inc_buffer_size(&self) {
        INC_BUFFER_SIZE.store(0, Ordering::Relaxed)
    }

    /// Atomically updates the reference count of object `o` by applying `f` to its current
    /// value, following the same semantics as `AtomicU8::fetch_update`.
    pub fn fetch_update(
        &self,
        o: ObjectReference,
        f: impl FnMut(u8) -> Option<u8>,
    ) -> Result<u8, u8> {
        RC_TABLE.fetch_update_atomic(o.to_raw_address(), Ordering::Relaxed, Ordering::Relaxed, f)
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
        RC_TABLE.store_atomic(o.to_raw_address(), count, Ordering::Relaxed)
    }

    /// Sets object `o`'s reference count to `count` using a non-atomic store, for use where the
    /// caller can guarantee there is no concurrent access.
    pub fn set_relaxed(&self, o: ObjectReference, count: u8) {
        unsafe { RC_TABLE.store(o.to_raw_address(), count) }
    }

    /// Returns object `o`'s current reference count.
    pub fn count(&self, o: ObjectReference) -> u8 {
        RC_TABLE.load_atomic(o.to_raw_address(), Ordering::Relaxed)
    }

    /// Returns `true` if the RC table entry at `o`'s address is zero. Used for both individual
    /// objects and line-granularity entries (e.g. straddle line markers), which share the same table.
    pub fn object_or_line_is_dead(&self, o: ObjectReference) -> bool {
        RC_TABLE.load_byte(o.to_raw_address()) == 0
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
        let v: u8 = RC_TABLE.load_atomic(o.to_raw_address(), Ordering::Relaxed);
        v == 0
    }

    /// Returns `true` if object `o`'s reference count is zero (dead) or has saturated at
    /// `MAX_REF_COUNT` (sticky).
    pub fn is_dead_or_stuck(&self, o: ObjectReference) -> bool {
        let v: u8 = RC_TABLE.load_atomic(o.to_raw_address(), Ordering::Relaxed);
        v == 0 || v == MAX_REF_COUNT
    }

    /// Returns `true` if the granule at `a` carries a synthetic line-occupancy mark rather than an
    /// object of its own -- that is, some object's allocation covers it without starting there.
    ///
    /// This is the per-granule query. `RC_STRADDLE_LINES` used to be per line, which could not
    /// describe a mark sitting at an object's header word, and was cleared by whichever of two
    /// objects sharing a line died first. See `set_occupied_line_marks`.
    pub fn is_straddle_granule(&self, a: Address) -> bool {
        let v: u8 = unsafe { RC_STRADDLE_LINES.load::<u8>(a) };
        v != 0
    }

    /// Returns `true` if `line` begins with a synthetic occupancy mark, i.e. an object covers its
    /// start without beginning there.
    pub fn is_straddle_line(&self, line: Line) -> bool {
        self.is_straddle_granule(line.start())
    }

    /// Returns `true` if address `a` holds a synthetic occupancy mark, i.e. it is covered by an
    /// object that starts elsewhere and so is not itself something to trace or sweep.
    pub fn address_is_in_straddle_line(&self, a: Address) -> bool {
        self.count(a.to_object_reference::<VM>()) != 0 && self.is_straddle_granule(a)
    }

    /// Marks every line (other than the one holding the object's own count) spanned by object
    /// `o` as occupied and straddled, so the object can be identified from any of them.
    pub fn mark_straddle_object(&self, o: ObjectReference) {
        let size = VM::VMObjectModel::get_current_size(o);
        self.set_occupied_line_marks::<true>(o, size)
    }

    /// Clears the straddle-line and reference-count markers set by `mark_straddle_object`.
    ///
    /// This used to clear only the lines beyond the first, and only for objects longer than a
    /// line, while `set_occupied_line_marks` marked a wider set: the header's line, and any line
    /// a *sub-line* object ran into by crossing a boundary. Those extra marks were then never
    /// paired with a straddle bit, so `SweepDeadCycles` could not recognise them as synthetic
    /// and tried to size them as objects. Both directions now go through the one function.
    pub fn unmark_straddle_object(&self, o: ObjectReference) {
        let size = VM::VMObjectModel::get_current_size(o);
        self.set_occupied_line_marks::<false>(o, size);
    }

    /// Debug assertion that every `MIN_OBJECT_SIZE` granule within object `o` has a reference
    /// count of zero, used to verify that a reclaimed object has been fully cleared.
    pub fn assert_zero_ref_count(&self, o: ObjectReference) {
        let size = VM::VMObjectModel::get_current_size(o);
        for i in (0..size).step_by(MIN_OBJECT_SIZE) {
            let a = o.to_raw_address() + i;
            assert_eq!(0, self.count(a.to_object_reference::<VM>()));
        }
    }

    /// Called when object `o` is promoted to mature space; records the lines it occupies,
    /// deriving its size from the VM binding.
    pub fn promote(&self, o: ObjectReference) {
        self.promote_with_size(o, o.get_size::<VM>())
    }

    /// Same as `promote`, but with the object's size supplied by the caller instead of being
    /// queried from the VM binding.
    pub fn promote_with_size(&self, o: ObjectReference, size: usize) {
        // Covers the straddle bits too, for every line the object touches rather than only for
        // objects longer than a line; see `set_occupied_line_marks`.
        self.set_occupied_line_marks::<true>(o, size);
    }

    /// Give every line an object occupies a non-zero count, except the one already
    /// holding the object's own count, and mark those lines as straddled.
    ///
    /// The hole finder treats a line as free when all of its counts are zero, so every
    /// line an object touches needs one. Only the line containing the object's reference
    /// address gets one naturally. That leaves two gaps: the line holding the header,
    /// when the VM places the reference address after the start of the allocation (Julia
    /// does), and any further line the object runs into. The latter is not limited to
    /// objects longer than a line — a 208-byte object crossing a boundary spans two.
    ///
    /// Each such count is *synthetic*: it stands for "this line is occupied", not for an
    /// object beginning there. `SweepDeadCycles` linear-scans the reference-count table and
    /// would otherwise take one for an object start and try to size it, reading a type tag
    /// from whatever precedes it.
    ///
    /// So each synthetic count is paired with a straddle bit at *the mark's own address*, which is
    /// what tells the sweeper the two apart. `RC_STRADDLE_LINES` is per granule for this reason,
    /// not per line: two objects can touch one line -- one object's header mark can share a line
    /// with another object's body -- so a per-line bit is cleared by whichever dies first and
    /// orphans the other's mark. That is exactly how a stray count outlived its straddle bit and
    /// crashed the sweep.
    ///
    /// Encoding the marks as `MAX_REF_COUNT` and having the sweeper skip that value also works to
    /// stop the crash, but it is wrong: it conflates a mark with a genuinely sticky object, and
    /// skipping those removes the only path that ever reclaims them, since cycle collection is
    /// what is supposed to free an object whose count saturated. It cost 287 collections and
    /// 364,584 reserved pages where this costs 151 and ~24,000.
    ///
    /// Marking and unmarking run through here with `MARK` toggled so the two can never cover
    /// different sets of lines.
    fn set_occupied_line_marks<const MARK: bool>(&self, o: ObjectReference, size: usize) {
        let start = VM::VMObjectModel::ref_to_object_start(o);
        let ref_line = Line::from_unaligned_address(o.to_raw_address());
        let first_line = Line::from_unaligned_address(start);
        // The end is exclusive, so it has to be the line *after* the one holding the
        // object's last byte. Rounding the end address down instead leaves the final line
        // unmarked whenever the object does not finish exactly on a line boundary: its
        // counts stay zero, the hole finder reads the line as free, and the allocator hands
        // out memory the object is still occupying.
        let end_line = Line::from_unaligned_address(start + (size - 1)).next();
        let v = if MARK { 1u8 } else { 0u8 };
        // The first line's start may precede the object and so may belong to a different
        // object; record against the header word instead, which cannot. Every later line
        // begins inside this object, so its start is safe to use.
        if first_line != ref_line {
            unsafe { RC_TABLE.store(start, v) };
            unsafe { RC_STRADDLE_LINES.store(start, v) };
        }
        let mut line = first_line.next();
        while line != end_line {
            if line != ref_line {
                unsafe { RC_TABLE.store(line.start(), v) };
                unsafe { RC_STRADDLE_LINES.store(line.start(), v) };
            }
            line = line.next();
        }
    }
}

impl<VM: VMBinding> Clone for RefCountHelper<VM> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}
