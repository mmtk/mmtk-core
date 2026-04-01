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

pub const LOG_REF_COUNT_BITS: usize = 1;
pub const REF_COUNT_BITS: u8 = 1 << LOG_REF_COUNT_BITS;
pub const REF_COUNT_MASK: u8 = (((1u16 << REF_COUNT_BITS) - 1) & 0xff) as u8;
pub const MAX_REF_COUNT: u8 = REF_COUNT_MASK;

pub const LOG_MIN_OBJECT_SIZE: usize = crate::util::constants::LOG_MIN_OBJECT_SIZE as _;
pub const MIN_OBJECT_SIZE: usize = 1 << LOG_MIN_OBJECT_SIZE;

pub const RC_STRADDLE_LINES: SideMetadataSpec =
    crate::util::metadata::side_metadata::spec_defs::RC_STRADDLE_LINES;

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

#[repr(transparent)]
#[derive(Debug, Copy)]
pub struct RefCountHelper<VM: VMBinding>(PhantomData<VM>);

impl<VM: VMBinding> RefCountHelper<VM> {
    pub const NEW: Self = Self(PhantomData);

    pub fn inc_buffer_size(&self) -> usize {
        INC_BUFFER_SIZE.load(Ordering::Relaxed)
    }

    pub fn increase_inc_buffer_size(&self, delta: usize) {
        INC_BUFFER_SIZE.store(
            INC_BUFFER_SIZE
                .load(Ordering::Relaxed)
                .saturating_add(delta),
            Ordering::Relaxed,
        );
    }

    pub fn reset_inc_buffer_size(&self) {
        INC_BUFFER_SIZE.store(0, Ordering::Relaxed)
    }

    pub fn fetch_update(
        &self,
        o: ObjectReference,
        f: impl FnMut(u8) -> Option<u8>,
    ) -> Result<u8, u8> {
        RC_TABLE.fetch_update_atomic(o.to_raw_address(), Ordering::Relaxed, Ordering::Relaxed, f)
    }

    pub fn is_stuck(&self, o: ObjectReference) -> bool {
        self.count(o) == MAX_REF_COUNT
    }

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

    pub fn inc(&self, o: ObjectReference) -> Result<u8, u8> {
        self.fetch_update(o, |x| {
            debug_assert!(x <= MAX_REF_COUNT);
            if x == MAX_REF_COUNT {
                None
            } else {
                Some(x + 1)
            }
        })
    }

    pub fn dec(&self, o: ObjectReference) -> Result<u8, u8> {
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

    pub fn set(&self, o: ObjectReference, count: u8) {
        RC_TABLE.store_atomic(o.to_raw_address(), count, Ordering::Relaxed)
    }

    pub fn set_relaxed(&self, o: ObjectReference, count: u8) {
        unsafe { RC_TABLE.store(o.to_raw_address(), count) }
    }

    pub fn count(&self, o: ObjectReference) -> u8 {
        RC_TABLE.load_atomic(o.to_raw_address(), Ordering::Relaxed)
    }

    pub fn prefetch_read(&self, o: ObjectReference) {
        RC_TABLE.prefetch_read(o.to_raw_address())
    }

    pub fn prefetch_write(&self, o: ObjectReference) {
        RC_TABLE.prefetch_write(o.to_raw_address())
    }

    pub fn object_or_line_is_dead(&self, o: ObjectReference) -> bool {
        RC_TABLE.load_byte(o.to_raw_address()) == 0
    }

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

    #[allow(unused)]
    pub fn is_dead(&self, o: ObjectReference) -> bool {
        let v: u8 = RC_TABLE.load_atomic(o.to_raw_address(), Ordering::Relaxed);
        v == 0
    }

    pub fn is_dead_or_stuck(&self, o: ObjectReference) -> bool {
        let v: u8 = RC_TABLE.load_atomic(o.to_raw_address(), Ordering::Relaxed);
        v == 0 || v == MAX_REF_COUNT
    }

    pub fn is_straddle_line(&self, line: Line) -> bool {
        let v: u8 = unsafe { RC_STRADDLE_LINES.load::<u8>(line.start()) };
        v != 0
    }

    pub fn address_is_in_straddle_line(&self, a: Address) -> bool {
        let line = Line::from(Line::align(a));
        self.count(a.to_object_reference::<VM>()) != 0 && self.is_straddle_line(line)
    }

    fn mark_straddle_object_with_size(&self, o: ObjectReference, size: usize) {
        debug_assert!(size > Line::BYTES);
        let start_line = Line::containing::<VM>(o).next();
        let end_line = Line::from(Line::align(o.to_raw_address() + size));
        let mut line = start_line;
        while line != end_line {
            unsafe { RC_STRADDLE_LINES.store(line.start(), 1u8) };
            self.set_relaxed(line.start().to_object_reference::<VM>(), 1);
            line = line.next();
        }
    }

    pub fn mark_straddle_object(&self, o: ObjectReference) {
        let size = VM::VMObjectModel::get_current_size(o);
        self.mark_straddle_object_with_size(o, size)
    }

    pub fn unmark_straddle_object(&self, o: ObjectReference) {
        // debug_assert!(crate::args::RC_NURSERY_EVACUATION);
        let size = VM::VMObjectModel::get_current_size(o);
        if size > Line::BYTES {
            let start_line = Line::containing::<VM>(o).next();
            let end_line = Line::from(Line::align(o.to_raw_address() + size));
            let mut line = start_line;
            while line != end_line {
                self.set_relaxed(line.start().to_object_reference::<VM>(), 0);
                // std::sync::atomic::fence(Ordering::Relaxed);
                unsafe { RC_STRADDLE_LINES.store(line.start(), 0u8) };
                // std::sync::atomic::fence(Ordering::Relaxed);
                line = line.next();
            }
        }
    }

    pub fn assert_zero_ref_count(&self, o: ObjectReference) {
        let size = VM::VMObjectModel::get_current_size(o);
        for i in (0..size).step_by(MIN_OBJECT_SIZE) {
            let a = o.to_raw_address() + i;
            assert_eq!(0, self.count(a.to_object_reference::<VM>()));
        }
    }

    pub fn promote(&self, o: ObjectReference) {
        o.log_start_address::<VM>();
        let size = o.get_size::<VM>();
        if size > Line::BYTES {
            self.mark_straddle_object_with_size(o, size);
        }
    }

    pub fn promote_with_size(&self, o: ObjectReference, size: usize) {
        o.log_start_address::<VM>();
        if size > Line::BYTES {
            self.mark_straddle_object_with_size(o, size);
        }
    }
}

impl<VM: VMBinding> Clone for RefCountHelper<VM> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}
