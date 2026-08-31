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

    /// Sets the reference count for the line containing object `o` to `count` using a non-atomic store,
    /// for use where the caller can guarantee there is no concurrent access.
    pub fn set_line_relaxed(&self, line: Line, count: u8) {
        unsafe { RC_TABLE.store(line.start(), count) }
    }

    /// Returns object `o`'s current reference count.
    pub fn count(&self, o: ObjectReference) -> u8 {
        RC_TABLE.load_atomic(o.to_raw_address(), Ordering::Relaxed)
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

    /// The bit within a line's `RC_STRADDLE_LINES` value that records occupancy at address `a`,
    /// or `None` if a mark can never sit at `a`'s position within its line.
    ///
    /// Bit 0 covers a line's own start: the mark used when a straddling object's body fully or
    /// partially spans the line (see `mark_straddle_object_with_size`). Bits `1..=K`, where `K =
    /// ceil(OBJECT_REF_OFFSET_UPPER_BOUND / MIN_OBJECT_SIZE)`, cover the `K` granules trailing a
    /// line, counting backward from the last granule (bit 1) -- the only positions a header mark
    /// can land on when the VM's reference address trails the allocation start by up to
    /// `OBJECT_REF_OFFSET_UPPER_BOUND` bytes (see the header case in
    /// `mark_straddle_object_with_size`). Every other granule in a line can never hold a mark of
    /// either kind, so it is always treated as a real object with no metadata lookup at all.
    ///
    /// For a VM with `OBJECT_REF_OFFSET_UPPER_BOUND == 0` (the default: reference address always
    /// at or ahead of allocation start by a fixed, non-negative amount), `K == 0` and only bit 0
    /// is ever used.
    fn straddle_bit(a: Address) -> Option<usize> {
        let line = Line::from_unaligned_address(a);
        let offset_in_line = a - line.start();
        if offset_in_line == 0 {
            return Some(0);
        }
        let trailing_granules = (VM::VMObjectModel::OBJECT_REF_OFFSET_UPPER_BOUND as usize)
            .saturating_add(MIN_OBJECT_SIZE - 1)
            / MIN_OBJECT_SIZE;
        let dist_from_end = (Line::BYTES - offset_in_line) / MIN_OBJECT_SIZE;
        (dist_from_end >= 1 && dist_from_end <= trailing_granules).then_some(dist_from_end)
    }

    /// Returns `true` if object `o` is in a straddle line. The function does not check rc table.
    ///
    /// When `UNIFIED_OBJECT_REFERENCE_ADDRESS` is true (reference address == allocation start,
    /// true of most VMs), only a line's own start address can ever hold a mark (see
    /// `mark_straddle_object_with_size`), so this checks `o`'s address is exactly a line start
    /// before trusting the byte there: since that function marks a straddling object's tail line
    /// even when the object does not fill it, a marked line can still hold an unrelated, ordinary
    /// object elsewhere within it, and that object's own reference address must not be misread as
    /// a straddle mark.
    ///
    /// Otherwise (the reference address can trail the allocation start, e.g. Julia), a mark can
    /// also sit at one of the few granules trailing a line -- see `straddle_bit`, which rules out
    /// every other position with no metadata access at all -- and multiple independent marks can
    /// land in the same line's byte, so bits are read individually rather than as a whole byte.
    pub fn object_is_in_straddle_line_no_rc_check(&self, o: ObjectReference) -> bool {
        let a = o.to_raw_address();
        if VM::VMObjectModel::UNIFIED_OBJECT_REFERENCE_ADDRESS {
            if !Line::is_aligned(a) {
                return false;
            }
            return unsafe { RC_STRADDLE_LINES.load::<u8>(a) != 0 };
        }
        let Some(bit) = Self::straddle_bit(a) else {
            return false;
        };
        let line = Line::from_unaligned_address(a);
        let v: u8 = RC_STRADDLE_LINES.load_atomic(line.start(), Ordering::Relaxed);
        (v >> bit) & 1 != 0
    }

    /// Returns `true` if address `a` falls within a live object whose containing line is marked
    /// as a straddle line.
    ///
    /// Delegates to `object_is_in_straddle_line_no_rc_check` rather than re-deriving `line.start()`
    /// and checking the byte directly: `count(o) != 0` alone does not establish that `o` itself is
    /// the mark, only that *some* object at `o`'s address is live, and that object can be a
    /// genuinely different, ordinary object sharing a marked line with a straddling object's tail
    /// (see `mark_straddle_object_with_size`) -- checking the byte without checking `o`'s own
    /// position would misclassify that unrelated object as a straddle continuation.
    pub fn object_is_in_straddle_line(&self, o: ObjectReference) -> bool {
        self.count(o) != 0 && self.object_is_in_straddle_line_no_rc_check(o)
    }

    /// Marks (`MARK == true`) or unmarks (`MARK == false`) every line an object occupies other
    /// than the one holding its own count, so the object can be identified from any of them.
    /// `mark_straddle_object`/`promote*` and `unmark_straddle_object` share this one
    /// implementation so the two directions can never cover different sets of lines -- they used
    /// to be separate, near-duplicate loops, and drifted: one covered a wider set of lines than
    /// the other, leaving marks that were set but never cleared.
    ///
    /// The hole finder treats a line as free when all of its counts are zero, so every line an
    /// object touches needs one. Only the line containing the object's reference address gets one
    /// naturally. That leaves two gaps: the line holding the header, when the VM places the
    /// reference address after the start of the allocation (Julia does), and any further line the
    /// object runs into. The latter is not limited to objects longer than a line -- a small object
    /// positioned near the end of a line can have its header in one line and its reference address
    /// in the next -- which is why this does not guard on `size > Line::BYTES`.
    ///
    /// Each such count is *synthetic*: it stands for "this line is occupied", not for an object
    /// beginning there. `SweepDeadCycles` linear-scans the reference-count table and would
    /// otherwise take one for an object start and try to size it, reading a type tag from
    /// whatever precedes it. So each synthetic count is paired with a straddle bit at *the mark's
    /// own address*, which is what tells the sweeper the two apart. That bit is set and cleared
    /// with an atomic fetch_or/fetch_and on `RC_STRADDLE_LINES` (see `straddle_bit`), never a
    /// plain store: two objects can touch one line -- one object's header mark can share a line
    /// with another object's real reference address, or with a second, unrelated header mark --
    /// and each needs its own bit within the line's value, independently settable and clearable,
    /// or one dying object's unmark would clobber a still-live one's mark (or a live object's own
    /// reference address could be misread as synthetic). That is exactly how a stray count
    /// outlived its straddle bit and crashed the sweep before the marks were made independent.
    ///
    /// Encoding the marks as `MAX_REF_COUNT` and having the sweeper skip that value also works to
    /// stop the crash, but it is wrong: it conflates a mark with a genuinely sticky object, and
    /// skipping those removes the only path that ever reclaims them, since cycle collection is
    /// what is supposed to free an object whose count saturated. It cost 287 collections and
    /// 364,584 reserved pages where this costs 151 and ~24,000.
    fn mark_straddle_object_with_size<const MARK: bool>(&self, o: ObjectReference, size: usize) {
        let start = o.to_object_start::<VM>();
        let ref_line = Line::from_unaligned_address(o.to_raw_address());
        let first_line = Line::from_unaligned_address(start);
        // The end is exclusive, so it must be the line *after* the one holding the object's
        // last byte. Rounding `start + size` down to a line (as it would be if the object does
        // not end exactly on a line boundary) leaves that last line unmarked, and its RC table
        // entry stays zero. `rc_get_next_available_lines` only skips the *first* line of a hole
        // it finds, on the assumption that the line before a hole may hold the tail of a
        // straddling object; it does not know this specific line is that tail, so once this
        // block is reused enough times that this line ends up as the second line of some hole
        // instead of the first, it is handed out as free while the object is still live in it.
        let end_line = Line::from_unaligned_address(start + size - 1).next();
        let v = if MARK { 1u8 } else { 0u8 };
        // The first line's start may precede the object (and so may belong to a different,
        // preceding object); record against the header word instead, which cannot. Every later
        // line begins inside this object, so its own start is always safe to use.
        if first_line != ref_line {
            unsafe { RC_TABLE.store(start, v) };
            self.set_straddle_bit::<MARK>(start);
        }
        let mut line = first_line.next();
        while line != end_line {
            if line != ref_line {
                self.set_line_relaxed(line, v);
                self.set_straddle_bit::<MARK>(line.start());
            }
            line = line.next();
        }
    }

    /// Marks every line (other than the one holding its reference count) spanned by object `o` as
    /// a straddle line, so the object can be identified from any of the lines it touches.
    pub fn mark_straddle_object(&self, o: ObjectReference) {
        let size = VM::VMObjectModel::get_current_size(o);
        self.mark_straddle_object_with_size::<true>(o, size)
    }

    /// Clears the straddle-line and reference-count markers set by `mark_straddle_object` for
    /// every line (other than the one holding its reference count) spanned by object `o`.
    pub fn unmark_straddle_object(&self, o: ObjectReference) {
        // debug_assert!(crate::args::RC_NURSERY_EVACUATION);
        let size = VM::VMObjectModel::get_current_size(o);
        self.mark_straddle_object_with_size::<false>(o, size);
    }

    /// Set or clear the mark that records occupancy at `a`.
    ///
    /// Only called from `mark_straddle_object_with_size` with an address `straddle_bit` is
    /// guaranteed to resolve: `line.start()` always resolves (bit 0), and `start` (the header
    /// case) only reaches here when `first_line != ref_line`, which by the
    /// `OBJECT_REF_OFFSET_UPPER_BOUND` invariant means `start` is within the trailing granules
    /// `straddle_bit` covers.
    ///
    /// When `UNIFIED_OBJECT_REFERENCE_ADDRESS` is true, `mark_straddle_object_with_size` never
    /// takes the header case, so `a` is always exactly `line.start()` (bit 0) and the whole byte
    /// is the mark -- a plain, non-atomic store, same as the query side's plain load.
    ///
    /// Otherwise, multiple independent marks (a header mark and/or a tail-line mark, from two
    /// different objects) can land in the same line's byte, at different bits, so each bit is set
    /// and cleared with an atomic fetch_or/fetch_and rather than a plain store: one object's
    /// unmark must not clobber a different, still-live object's mark in the same byte.
    fn set_straddle_bit<const MARK: bool>(&self, a: Address) {
        if VM::VMObjectModel::UNIFIED_OBJECT_REFERENCE_ADDRESS {
            unsafe { RC_STRADDLE_LINES.store::<u8>(a, if MARK { 1u8 } else { 0u8 }) };
            return;
        }
        let bit = Self::straddle_bit(a).unwrap();
        let line = Line::from_unaligned_address(a);
        let mask = 1u8 << bit;
        if MARK {
            RC_STRADDLE_LINES.fetch_or_atomic::<u8>(line.start(), mask, Ordering::Relaxed);
        } else {
            RC_STRADDLE_LINES.fetch_and_atomic::<u8>(line.start(), !mask, Ordering::Relaxed);
        }
    }

    /// Debug assertion that every `MIN_OBJECT_SIZE` granule within object `o` has a reference
    /// count of zero, used to verify that a reclaimed object has been fully cleared.
    pub fn assert_zero_ref_count(&self, o: ObjectReference) {
        let size = VM::VMObjectModel::get_current_size(o);
        for i in (0..size).step_by(MIN_OBJECT_SIZE) {
            let a = o.to_raw_address() + i;
            assert_eq!(0, self.count_by_address(a));
        }
    }

    /// Called when object `o` is promoted to mature space; marks every line it touches other
    /// than the one holding its reference count, deriving its size from the VM binding.
    pub fn promote(&self, o: ObjectReference) {
        let size = o.get_size::<VM>();
        self.mark_straddle_object_with_size::<true>(o, size);
    }

    /// Same as `promote`, but with the object's size supplied by the caller instead of being
    /// queried from the VM binding.
    pub fn promote_with_size(&self, o: ObjectReference, size: usize) {
        self.mark_straddle_object_with_size::<true>(o, size);
    }
}

impl<VM: VMBinding> Clone for RefCountHelper<VM> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}
