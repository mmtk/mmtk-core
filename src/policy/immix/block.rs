use super::defrag::Histogram;
use super::line::{Line, RCArray};
use super::ImmixSpace;
use crate::util::heap::blockpageresource_legacy::BlockPool;
use crate::util::heap::chunk_map::Chunk;
use crate::util::linear_scan::{Region, RegionIterator};
use crate::util::metadata::side_metadata::spec_defs::{BLOCK_IN_USE, BLOCK_OWNER};
use crate::util::metadata::side_metadata::*;
#[cfg(feature = "vo_bit")]
use crate::util::metadata::vo_bit;
use crate::util::object_enum::BlockMayHaveObjects;
use crate::util::{constants::*, OpaquePointer, VMThread};
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use std::sync::atomic::{AtomicU8, Ordering};

/// The block allocation state.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum BlockState {
    /// the block is not allocated.
    Unallocated,
    /// the block is a young block.
    Unmarked,
    /// the block is allocated and marked.
    Marked,
    /// the block is marked as reusable.
    Reusable { unavailable_lines: u8 },
}

impl BlockState {
    /// Private constant
    const MARK_UNALLOCATED: u8 = 0;
    /// Private constant
    const MARK_UNMARKED: u8 = u8::MAX;
    /// Private constant
    const MARK_MARKED: u8 = u8::MAX - 1;
}

impl From<u8> for BlockState {
    fn from(state: u8) -> Self {
        match state {
            Self::MARK_UNALLOCATED => BlockState::Unallocated,
            Self::MARK_UNMARKED => BlockState::Unmarked,
            Self::MARK_MARKED => BlockState::Marked,
            unavailable_lines => BlockState::Reusable { unavailable_lines },
        }
    }
}

impl From<BlockState> for u8 {
    fn from(state: BlockState) -> Self {
        match state {
            BlockState::Unallocated => BlockState::MARK_UNALLOCATED,
            BlockState::Unmarked => BlockState::MARK_UNMARKED,
            BlockState::Marked => BlockState::MARK_MARKED,
            BlockState::Reusable { unavailable_lines } => {
                assert_ne!(unavailable_lines, 0);
                u8::min(unavailable_lines, u8::MAX - 4)
            }
        }
    }
}

impl BlockState {
    /// Test if the block is reuasable.
    pub const fn is_reusable(&self) -> bool {
        matches!(self, BlockState::Reusable { .. })
    }
}

/// Data structure to reference an OS 4K page.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq)]
pub struct Page(Address);

impl Region for Page {
    const LOG_BYTES: usize = LOG_BYTES_IN_PAGE as usize;

    fn from_aligned_address(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    fn start(&self) -> Address {
        self.0
    }
}

/// Data structure to reference an immix block.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq)]
pub struct Block(Address);

impl Region for Block {
    #[cfg(not(feature = "immix_smaller_block"))]
    const LOG_BYTES: usize = {
        if cfg!(feature = "lxr_block_16k") {
            14
        } else if cfg!(feature = "lxr_block_32k") {
            15
        } else if cfg!(feature = "lxr_block_64k") {
            16
        } else if cfg!(feature = "lxr_block_128k") {
            17
        } else if cfg!(feature = "lxr_block_256k") {
            18
        } else if cfg!(feature = "lxr_block_512k") {
            19
        } else if cfg!(feature = "lxr_block_1m") {
            20
        } else {
            15
        }
    };
    #[cfg(feature = "immix_smaller_block")]
    const LOG_BYTES: usize = 13;

    const BPR_ALLOC_TABLE: Option<SideMetadataSpec> =
        Some(crate::util::metadata::side_metadata::spec_defs::IX_BLOCK_ALLOC_BITS);

    fn from_aligned_address(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    fn start(&self) -> Address {
        self.0
    }
}

static GLOBAL_PHASE_EPOCH: AtomicU8 = AtomicU8::new(1);

impl BlockMayHaveObjects for Block {
    fn may_have_objects(&self) -> bool {
        self.get_state() != BlockState::Unallocated
    }
}

impl Block {
    /// Log bytes in block
    pub const LOG_BYTES: usize = <Self as Region>::LOG_BYTES;
    /// Bytes in block
    pub const BYTES: usize = 1 << Self::LOG_BYTES;
    /// Log pages in block
    pub const LOG_PAGES: usize = Self::LOG_BYTES - LOG_BYTES_IN_PAGE as usize;
    /// Pages in block
    pub const PAGES: usize = 1 << Self::LOG_PAGES;
    /// Log lines in block
    pub const LOG_LINES: usize = Self::LOG_BYTES - Line::LOG_BYTES;
    /// Lines in block
    pub const LINES: usize = 1 << Self::LOG_LINES;

    /// Block defrag state table (side)
    pub const DEFRAG_STATE_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::IX_BLOCK_DEFRAG;

    /// Block mark table (side)
    pub const MARK_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::IX_BLOCK_MARK;
    pub const LOG_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::IX_BLOCK_LOG;
    pub const NURSERY_PROMOTION_STATE_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::NURSERY_PROMOTION_STATE;
    pub const PHASE_EPOCH: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::PHASE_EPOCH;

    pub fn calc_dead_lines(&self) -> usize {
        let mut dead_lines = 0;
        let rc_array = RCArray::of(*self);
        // let mut skip_next_dead = false;
        for i in 0..Self::LINES {
            if rc_array.is_dead(i) {
                // if i == 0 {
                //     dead_lines += 1;
                // } else if skip_next_dead {
                //     skip_next_dead = false;
                // } else {
                //     dead_lines += 1;
                // }
                dead_lines += 1;
            } else {
                // skip_next_dead = true;
            }
        }
        dead_lines
    }

    pub const ZERO: Self = Self(Address::ZERO);

    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    /// Align the address to a block boundary.
    pub const fn align(address: Address) -> Address {
        address.align_down(Self::BYTES)
    }

    /// Get the block from a given address.
    /// The address must be block-aligned.
    pub fn from(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    pub fn of(a: Address) -> Self {
        Self::from(Self::align(a))
    }

    /// Get the block containing the given address.
    /// The input address does not need to be aligned.
    pub fn containing(object: ObjectReference) -> Self {
        Self(object.to_raw_address().align_down(Self::BYTES))
    }

    /// Get block start address
    pub const fn start(&self) -> Address {
        self.0
    }

    /// Get block end address
    pub const fn end(&self) -> Address {
        self.0.add(Self::BYTES)
    }

    /// Get the chunk containing the block.
    pub fn chunk(&self) -> Chunk {
        Chunk::from_unaligned_address(self.0)
    }

    /// Get the address range of the block's line mark table.
    #[allow(clippy::assertions_on_constants)]
    pub fn line_mark_table(&self) -> MetadataByteArrayRef<{ Block::LINES }> {
        debug_assert!(!super::BLOCK_ONLY);
        MetadataByteArrayRef::<{ Block::LINES }>::new(&Line::MARK_TABLE, self.start(), Self::BYTES)
    }

    fn try_lock(&self) -> bool {
        let result = BLOCK_IN_USE.fetch_update_atomic::<u8, _>(
            self.start(),
            Ordering::Relaxed,
            Ordering::Relaxed,
            |b| {
                if b == 1 {
                    return None;
                }
                Some(1)
            },
        );
        result == Ok(0)
    }

    fn lock_skip_reusing_or_unallocated(&self) -> bool {
        loop {
            std::hint::spin_loop();
            let state = self.get_state();
            if state == BlockState::Unallocated || (Self::in_mutatar_phase() && self.is_reusing()) {
                return false;
            }
            let result = BLOCK_IN_USE.fetch_update_atomic::<u8, _>(
                self.start(),
                Ordering::Relaxed,
                Ordering::Relaxed,
                |b| {
                    if b == 1 {
                        return None;
                    }
                    Some(1)
                },
            );
            if result == Ok(0) {
                return true;
            }
        }
    }

    pub fn try_lock_with_condition(&self, predicate: impl Fn() -> bool) -> bool {
        if !predicate() {
            return false;
        }
        let locked = self.try_lock();
        if !locked {
            return false;
        }
        if !predicate() {
            self.unlock();
            return false;
        }
        true
    }

    pub fn is_locked(&self) -> bool {
        BLOCK_IN_USE.load_atomic::<u8>(self.start(), Ordering::Relaxed) != 0
    }

    pub fn unlock(&self) {
        BLOCK_IN_USE.store_atomic::<u8>(self.start(), 0u8, Ordering::Relaxed);
    }

    pub fn get_owner(&self) -> Option<VMThread> {
        let ptr = BLOCK_OWNER.load_atomic::<usize>(self.start(), Ordering::Relaxed);
        if ptr == 0 {
            None
        } else {
            Some(VMThread(OpaquePointer::from_mut_ptr(ptr as *mut ())))
        }
    }

    pub fn set_owner(&self, owner: Option<VMThread>) {
        let ptr = if let Some(owner) = owner {
            owner.0.to_address().as_usize()
        } else {
            0
        };
        BLOCK_OWNER.store_atomic(self.start(), ptr, Ordering::Relaxed);
    }

    /// The block is in one of the copy allocator's local block list.
    pub fn is_owned_by_copy_allocator(&self) -> bool {
        let ge = Self::global_phase_epoch();
        assert_eq!(ge & 1, 0);
        let e: u8 = self.phase_epoch();
        ge == e
    }

    /// The global phase epoch.
    /// This counter is bumped by one at the end of every mutator and GC phase.
    /// Any block matching this epoch are used for allocation in the current phase.
    pub fn global_phase_epoch() -> u8 {
        GLOBAL_PHASE_EPOCH.load(Ordering::Relaxed)
    }

    /// Get the current block phase epoch.
    /// This indicates the last phase that this block is used for object allocation.
    /// Either as a clean block or a partially-free block.
    ///
    /// Odd epoch means the block is in a mutator phase.
    /// Even epoch means the block is allocated in a GC phase.
    pub fn phase_epoch(&self) -> u8 {
        Self::PHASE_EPOCH.load_atomic::<u8>(self.start(), Ordering::Relaxed)
    }

    pub fn update_phase_epoch(&self) {
        Self::PHASE_EPOCH.store_atomic::<u8>(
            self.start(),
            Self::global_phase_epoch(),
            Ordering::Relaxed,
        );
    }

    pub fn is_reusing(&self) -> bool {
        self.get_state() != BlockState::Unallocated && self.is_nursery_or_reusing()
    }

    pub fn is_gc_reusing(&self) -> bool {
        if self.get_state() == BlockState::Unallocated {
            return false;
        }
        let ge = Self::global_phase_epoch();
        assert_eq!(ge & 1, 0);
        let e = self.phase_epoch();
        e == ge
    }

    pub fn is_nursery(&self) -> bool {
        self.get_state() == BlockState::Unallocated && self.is_nursery_or_reusing()
    }

    pub fn is_nursery_or_reusing(&self) -> bool {
        let ge = Self::global_phase_epoch();
        let e = self.phase_epoch();
        if (ge & 1) == 1 {
            return e == ge;
        } else {
            return e == ge - 1;
        }
    }

    fn in_mutatar_phase() -> bool {
        let ge = Self::global_phase_epoch();
        (ge & 1) == 1
    }

    pub fn update_global_phase_epoch<VM: VMBinding>(space: &ImmixSpace<VM>) {
        let old = GLOBAL_PHASE_EPOCH.load(Ordering::SeqCst);
        if old == 254 {
            GLOBAL_PHASE_EPOCH.store(1, Ordering::SeqCst);
            space.pr.reset_nursery_state();
        } else {
            GLOBAL_PHASE_EPOCH.store(old + 1, Ordering::SeqCst);
        }
    }

    pub fn is_reusable(&self) -> bool {
        if crate::args::RC_MATURE_EVACUATION && self.is_defrag_source() {
            return false;
        }
        self.get_state().is_reusable()
    }

    /// Get block mark state.
    pub fn get_state(&self) -> BlockState {
        let byte = Self::MARK_TABLE.load_atomic::<u8>(self.start(), Ordering::SeqCst);
        byte.into()
    }

    /// Set block mark state.
    pub fn set_state(&self, state: BlockState) {
        let state = u8::from(state);
        Self::MARK_TABLE.store_atomic::<u8>(self.start(), state, Ordering::SeqCst);
    }

    /// Set block mark state.
    pub fn fetch_update_state(
        &self,
        mut f: impl FnMut(BlockState) -> Option<BlockState>,
    ) -> Result<BlockState, BlockState> {
        Self::MARK_TABLE
            .fetch_update_atomic::<u8, _>(self.start(), Ordering::SeqCst, Ordering::SeqCst, |s| {
                f(s.into()).map(|x| u8::from(x))
            })
            .map(|x| (x as u8).into())
            .map_err(|x| (x as u8).into())
    }

    fn attempt_dealloc(&self) -> bool {
        self.fetch_update_state(|s| {
            if (Self::in_mutatar_phase() && self.is_reusing()) || s == BlockState::Unallocated {
                None
            } else {
                Some(BlockState::Unallocated)
            }
        })
        .is_ok()
    }

    // Defrag byte

    const DEFRAG_SOURCE_STATE: u8 = u8::MAX;

    /// Test if the block is marked for defragmentation.
    pub fn is_defrag_source(&self) -> bool {
        let byte = Self::DEFRAG_STATE_TABLE.load_byte(self.start());
        // The byte should be 0 (not defrag source) or 255 (defrag source) if this is a major defrag GC, as we set the values in PrepareBlockState.
        // But it could be any value in a nursery GC.
        byte != 0
    }

    pub fn in_defrag_block<VM: VMBinding>(o: ObjectReference) -> bool {
        Self::DEFRAG_STATE_TABLE.load_byte(o.to_raw_address()) != 0
    }

    pub fn address_in_defrag_block(a: Address) -> bool {
        Self::DEFRAG_STATE_TABLE.load_byte(a) != 0
    }

    /// Mark the block for defragmentation.
    pub fn set_as_defrag_source(&self, defrag: bool) {
        let byte = if defrag { Self::DEFRAG_SOURCE_STATE } else { 0 };
        Self::DEFRAG_STATE_TABLE.store_atomic::<u8>(self.start(), byte, Ordering::SeqCst);
    }

    /// Record the number of holes in the block.
    pub fn set_holes(&self, holes: usize) {
        Self::DEFRAG_STATE_TABLE.store_atomic::<u8>(self.start(), holes as u8, Ordering::SeqCst);
    }

    /// Get the number of holes.
    pub fn get_holes(&self) -> usize {
        let byte = Self::DEFRAG_STATE_TABLE.load_atomic::<u8>(self.start(), Ordering::SeqCst);
        debug_assert_ne!(byte, Self::DEFRAG_SOURCE_STATE);
        byte as usize
    }

    /// Initialize a clean block after acquired from page-resource.
    pub fn init<VM: VMBinding>(&self, copy: bool, reuse: bool, space: &ImmixSpace<VM>) {
        // println!("Alloc block {:?} copy={} reuse={}", self, copy, reuse);
        // #[cfg(feature = "sanity")]
        // if !copy && !reuse && space.rc_enabled {
        //     self.assert_log_table_cleared::<VM>(super::get_unlog_bit_slow::<VM>());
        // }
        self.update_phase_epoch();
        if space.rc_enabled {
            if !reuse {
                debug_assert_eq!(self.get_state(), BlockState::Unallocated);
            }
            self.clear_in_place_promoted();
            if copy {
                debug_assert!((self.phase_epoch() & 1) == 0);
            } else {
                debug_assert!((self.phase_epoch() & 1) != 0);
            }
            if copy {
                if reuse {
                    debug_assert!(!self.is_defrag_source());
                }
                self.set_state(BlockState::Unmarked);
                self.set_as_defrag_source(false);
            } else {
                if reuse {
                    debug_assert!(!self.is_defrag_source());
                } else {
                    debug_assert_eq!(self.get_state(), BlockState::Unallocated);
                    self.set_as_defrag_source(false);
                }
            }
        } else {
            self.set_state(if copy {
                BlockState::Marked
            } else {
                BlockState::Unmarked
            });
            if !reuse || cfg!(feature = "ix_no_defrag_fix") {
                Self::DEFRAG_STATE_TABLE.store_atomic::<u8>(self.start(), 0, Ordering::SeqCst);
            }
        }
    }

    /// Deinitalize a block before releasing.
    pub fn deinit<VM: VMBinding>(&self, space: &ImmixSpace<VM>) {
        self.set_state(BlockState::Unallocated);
        if space.rc_enabled {
            self.clear_in_place_promoted();
        }
        if space.rc_enabled {
            BLOCK_OWNER.store_atomic(self.start(), 0usize, Ordering::Relaxed);
            self.set_as_defrag_source(false);
        }
    }

    pub fn start_line(&self) -> Line {
        Line::from_aligned_address(self.start())
    }

    pub fn end_line(&self) -> Line {
        Line::from_aligned_address(self.end())
    }

    /// Get the range of lines within the block.
    #[allow(clippy::assertions_on_constants)]
    pub fn lines(&self) -> RegionIterator<Line> {
        debug_assert!(!super::BLOCK_ONLY);
        RegionIterator::<Line>::new(self.start_line(), self.end_line())
    }

    pub fn clear_rc_table<VM: VMBinding>(&self) {
        crate::util::rc::RC_TABLE.bzero_metadata(self.start(), Block::BYTES);
    }

    pub fn clear_striddle_table<VM: VMBinding>(&self) {
        crate::util::rc::RC_STRADDLE_LINES.bzero_metadata(self.start(), Block::BYTES);
    }

    #[allow(unused)]
    pub(super) fn clear_mark_table<VM: VMBinding>(&self) {
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
            .extract_side_spec()
            .bzero_metadata(self.start(), Self::BYTES);
    }

    pub(super) fn initialize_mark_table_as_marked<VM: VMBinding>(&self) {
        let meta = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.extract_side_spec();
        let start: *mut u8 = address_to_meta_address(&meta, self.start()).to_mut_ptr();
        let limit: *mut u8 = address_to_meta_address(&meta, self.end()).to_mut_ptr();
        unsafe {
            let bytes = limit.offset_from(start) as usize;
            std::ptr::write_bytes(start, 0xffu8, bytes);
        }
    }

    pub fn log(&self) -> bool {
        loop {
            let old_value: u8 = Self::LOG_TABLE.load_atomic(self.start(), Ordering::Relaxed);
            if old_value == 1 {
                return false;
            }
            if Self::LOG_TABLE
                .compare_exchange_atomic(self.start(), 0u8, 1u8, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return true;
            }
        }
    }

    pub fn set_as_in_place_promoted<VM: VMBinding>(&self, space: &ImmixSpace<VM>) {
        if self.is_in_place_promoted() {
            return;
        }
        loop {
            let old_value: u8 =
                Self::NURSERY_PROMOTION_STATE_TABLE.load_atomic(self.start(), Ordering::Relaxed);
            if old_value == 1 {
                return;
            }
            if Self::NURSERY_PROMOTION_STATE_TABLE
                .compare_exchange_atomic(self.start(), 0u8, 1u8, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                space
                    .block_allocation
                    .in_place_promoted_nursery_blocks
                    .fetch_add(1, Ordering::Relaxed);
                self.set_state(BlockState::Unmarked);
                self.update_phase_epoch();
                return;
            }
        }
    }

    pub fn is_in_place_promoted(&self) -> bool {
        Self::NURSERY_PROMOTION_STATE_TABLE.load_atomic::<u8>(self.start(), Ordering::Relaxed) != 0
    }

    fn clear_in_place_promoted(&self) {
        Self::NURSERY_PROMOTION_STATE_TABLE.store_atomic(self.start(), 0u8, Ordering::Relaxed);
    }

    pub fn unlog(&self) {
        Self::LOG_TABLE.store_atomic(self.start(), 0u8, Ordering::Relaxed);
    }

    pub fn clear_field_unlog_table<VM: VMBinding>(&self) {
        VM::VMObjectModel::GLOBAL_FIELD_UNLOG_BIT_SPEC
            .as_spec()
            .extract_side_spec()
            .bzero_metadata(self.start(), Block::BYTES);
    }

    pub fn assert_log_table_cleared<VM: VMBinding>(&self, meta: &SideMetadataSpec) {
        assert!(cfg!(debug_assertions) || cfg!(feature = "sanity"));
        let start = address_to_meta_address(meta, self.start()).to_ptr::<u128>();
        let limit = address_to_meta_address(meta, self.end()).to_ptr::<u128>();
        let table = unsafe { std::slice::from_raw_parts(start, limit.offset_from(start) as _) };
        for x in table {
            assert_eq!(*x, 0);
        }
    }

    pub fn initialize_field_unlog_table_as_unlogged<VM: VMBinding>(&self) {
        let meta = *VM::VMObjectModel::GLOBAL_FIELD_UNLOG_BIT_SPEC
            .as_spec()
            .extract_side_spec();
        let start: *mut u8 = address_to_meta_address(&meta, self.start()).to_mut_ptr();
        let limit: *mut u8 = address_to_meta_address(&meta, self.end()).to_mut_ptr();
        unsafe {
            let bytes = limit.offset_from(start) as usize;
            std::ptr::write_bytes(start, 0xffu8, bytes);
        }
    }

    pub fn rc_dead(&self) -> bool {
        type UInt = u128;
        const LOG_BITS_IN_UINT: usize =
            (std::mem::size_of::<UInt>() << 3).trailing_zeros() as usize;
        debug_assert!(
            Self::LOG_BYTES - crate::util::rc::LOG_MIN_OBJECT_SIZE
                + crate::util::rc::LOG_REF_COUNT_BITS
                >= LOG_BITS_IN_UINT
        );
        let start =
            address_to_meta_address(&crate::util::rc::RC_TABLE, self.start()).to_ptr::<UInt>();
        let limit =
            address_to_meta_address(&crate::util::rc::RC_TABLE, self.end()).to_ptr::<UInt>();
        let rc_table = unsafe { std::slice::from_raw_parts(start, limit.offset_from(start) as _) };
        for x in rc_table {
            if *x != 0 {
                return false;
            }
        }
        true
    }

    /// Sweep this block.
    /// Return true if the block is swept.
    pub fn sweep<VM: VMBinding>(
        &self,
        space: &ImmixSpace<VM>,
        mark_histogram: &mut Histogram,
        line_mark_state: Option<u8>,
    ) -> bool {
        self.set_as_defrag_source(false);
        if super::BLOCK_ONLY {
            match self.get_state() {
                BlockState::Unallocated => false,
                BlockState::Unmarked => {
                    #[cfg(feature = "vo_bit")]
                    vo_bit::helper::on_region_swept::<VM, _>(self, false);
                    unimplemented!();
                }
                BlockState::Marked => {
                    #[cfg(feature = "vo_bit")]
                    vo_bit::helper::on_region_swept::<VM, _>(self, true);

                    // The block is live.
                    false
                }
                _ => unreachable!(),
            }
        } else {
            // Calculate number of marked lines and holes.
            let mut marked_lines = 0;
            let mut holes = 0;
            let mut prev_line_is_marked = true;
            let line_mark_state = line_mark_state.unwrap();

            for line in self.lines() {
                if line.is_marked(line_mark_state) {
                    marked_lines += 1;
                    prev_line_is_marked = true;
                } else {
                    if prev_line_is_marked {
                        holes += 1;
                    }
                    // We need to clear the line mark state at least twice in every 128 GC
                    // otherwise, the line mark state of the last GC will stick around
                    if line_mark_state > Line::MAX_MARK_STATE - 2 {
                        line.mark(0);
                    }
                    #[cfg(feature = "immix_zero_on_release")]
                    crate::util::memory::zero(line.start(), Line::BYTES);

                    // We need to clear the pin bit if it is on the side, as this line can be reused
                    #[cfg(feature = "object_pinning")]
                    if let MetadataSpec::OnSide(side) = *VM::VMObjectModel::LOCAL_PINNING_BIT_SPEC {
                        side.bzero_metadata(line.start(), Line::BYTES);
                    }

                    prev_line_is_marked = false;
                }
            }

            if marked_lines == 0 {
                #[cfg(feature = "vo_bit")]
                vo_bit::helper::on_region_swept::<VM, _>(self, false);

                // Release the block if non of its lines are marked.
                space.release_block(*self);
                true
            } else {
                // There are some marked lines. Keep the block live.
                if marked_lines != Block::LINES {
                    // There are holes. Mark the block as reusable.
                    self.set_state(BlockState::Reusable {
                        unavailable_lines: usize::min(marked_lines, u8::MAX as usize) as _,
                    });
                } else {
                    // Clear mark state.
                    self.set_state(BlockState::Unmarked);
                }
                if cfg!(feature = "ix_live_size_based_defrag") {
                    // Update mark_histogram
                    mark_histogram[Block::LINES - marked_lines] += marked_lines;
                    // Record number of holes in block side metadata.
                    self.set_holes(Block::LINES - marked_lines);
                } else {
                    // Update mark_histogram
                    mark_histogram[holes] += marked_lines;
                    // Record number of holes in block side metadata.
                    self.set_holes(holes);
                }
                #[cfg(feature = "vo_bit")]
                vo_bit::helper::on_region_swept::<VM, _>(self, true);
                false
            }
        }
    }

    pub fn rc_sweep_mature<VM: VMBinding>(
        &self,
        space: &ImmixSpace<VM>,
        defrag: bool,
        rc_dead: bool,
    ) -> bool {
        if self.get_state() == BlockState::Unallocated {
            return false;
        }
        if defrag || rc_dead || self.rc_dead() {
            if !self.lock_skip_reusing_or_unallocated() {
                return false;
            }
            let dead = if defrag || self.attempt_dealloc() {
                self.deinit(space);
                true
            } else {
                false
            };
            self.unlock();
            return dead;
        }
        false
    }

    pub fn rc_table_start(&self) -> Address {
        address_to_meta_address(&crate::util::rc::RC_TABLE, self.start())
    }

    pub fn has_holes(&self) -> bool {
        let rc_array = RCArray::of(*self);
        let mut found_free_line = false;
        let mut free_lines = 0;
        for i in 0..Self::LINES {
            if rc_array.is_dead(i) {
                if i == 0 || found_free_line {
                    free_lines += 1
                } else if !found_free_line {
                    found_free_line = true;
                }
                if free_lines >= crate::args().min_reuse_lines {
                    return true;
                }
            } else {
                free_lines = 0;
                found_free_line = false;
            }
        }
        false
    }

    pub fn iter_holes(&self, mut f: impl FnMut(usize)) {
        let rc_array = RCArray::of(*self);
        let mut i = 0;
        while i < Block::LINES {
            if rc_array.is_dead(i) {
                let mut j = i + 1;
                while j < Block::LINES {
                    if !rc_array.is_dead(j) {
                        break;
                    }
                    j += 1;
                }
                f(j - i);
                i = j;
            } else {
                i += 1;
            }
        }
    }

    pub fn calc_holes(&self) -> usize {
        let rc_array = RCArray::of(*self);
        let search_next_hole = |start: usize| -> Option<usize> {
            // Find start
            let first_free_cursor = {
                let start_cursor = start;
                let mut first_free_cursor = None;
                let mut find_free_line = false;
                for i in start_cursor..Block::LINES {
                    if rc_array.is_dead(i) {
                        if i == 0 {
                            first_free_cursor = Some(i);
                            break;
                        } else if !find_free_line {
                            find_free_line = true;
                        } else {
                            first_free_cursor = Some(i);
                            break;
                        }
                    } else {
                        find_free_line = false;
                    }
                }
                first_free_cursor
            };
            let start = match first_free_cursor {
                Some(c) => c,
                _ => return None,
            };
            // Find limit
            let end = {
                let mut cursor = start + 1;
                while cursor < Block::LINES {
                    if !rc_array.is_dead(cursor) {
                        break;
                    }
                    cursor += 1;
                }
                cursor
            };
            Some(end)
        };
        let mut holes = 0;
        let mut cursor = 0;
        while let Some(end) = search_next_hole(cursor) {
            cursor = end;
            if end - cursor >= crate::args().min_reuse_lines {
                holes += 1;
            }
        }
        holes
    }
    /// Clear VO bits metadata for unmarked regions.
    /// This is useful for clearing VO bits during nursery GC for StickyImmix
    /// at which time young objects (allocated in unmarked regions) may die
    /// but we always consider old objects (in marked regions) as live.
    #[cfg(feature = "vo_bit")]
    pub fn clear_vo_bits_for_unmarked_regions(&self, line_mark_state: Option<u8>) {
        match line_mark_state {
            None => {
                match self.get_state() {
                    BlockState::Unmarked => {
                        // It may contain young objects.  Clear it.
                        vo_bit::bzero_vo_bit(self.start(), Self::BYTES);
                    }
                    BlockState::Marked => {
                        // It contains old objects.  Skip it.
                    }
                    _ => unreachable!(),
                }
            }
            Some(state) => {
                // With lines.
                for line in self.lines() {
                    if !line.is_marked(state) {
                        // It may contain young objects.  Clear it.
                        vo_bit::bzero_vo_bit(line.start(), Line::BYTES);
                    }
                }
            }
        }
    }
}

/// A non-block single-linked list to store blocks.
pub struct ReusableBlockPool {
    queue: BlockPool<Block>,
    num_workers: usize,
}

#[allow(unused)]
impl ReusableBlockPool {
    /// Create empty block list
    pub fn new(num_workers: usize) -> Self {
        Self {
            queue: BlockPool::new(num_workers),
            num_workers,
        }
    }

    /// Get number of blocks in this list.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Add a block to the list.
    pub fn push(&self, block: Block) {
        self.queue.push(block)
    }

    /// Pop a block out of the list.
    pub fn pop(&self) -> Option<Block> {
        self.queue.pop()
    }

    /// Clear the list.
    pub fn reset(&mut self) {
        self.queue = BlockPool::new(self.num_workers);
    }

    /// Iterate all the blocks in the queue. Call the visitor for each reported block.
    pub fn iterate_blocks(&self, mut f: impl FnMut(Block)) {
        self.queue.iterate_blocks(&mut f);
    }

    /// Flush the block queue
    pub fn flush_all(&self) {
        self.queue.flush_all();
    }
}
