use crate::util::conversions;
use crate::util::conversions::*;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::heap::layout::vm_layout_constants::MAX_CHUNKS;
use crate::util::Address;
use crate::util::ObjectReference;
#[cfg(debug_assertions)]
use crate::vm::VMBinding;

use super::sft::*;

pub struct SFTMap<'a> {
    sft: Vec<&'a (dyn SFT + Sync + 'static)>,
}

// TODO: MMTK<VM> holds a reference to SFTMap. We should have a safe implementation rather than use raw pointers for dyn SFT.
unsafe impl<'a> Sync for SFTMap<'a> {}

const EMPTY_SPACE_SFT: EmptySpaceSFT = EmptySpaceSFT {};

impl<'a> SFTMap<'a> {
    pub fn new() -> Self {
        SFTMap {
            sft: vec![&EMPTY_SPACE_SFT; MAX_CHUNKS],
        }
    }
    // This is a temporary solution to allow unsafe mut reference. We do not want several occurrence
    // of the same unsafe code.
    // FIXME: We need a safe implementation.
    #[allow(clippy::cast_ref_to_mut)]
    #[allow(clippy::mut_from_ref)]
    unsafe fn mut_self(&self) -> &mut Self {
        &mut *(self as *const _ as *mut _)
    }

    pub fn get(&self, address: Address) -> &'a dyn SFT {
        debug_assert!(address.chunk_index() < MAX_CHUNKS);
        let res = unsafe { *self.sft.get_unchecked(address.chunk_index()) };
        if DEBUG_SFT {
            trace!(
                "Get SFT for {} #{} = {}",
                address,
                address.chunk_index(),
                res.name()
            );
        }
        res
    }

    fn log_update(&self, space: &(dyn SFT + Sync + 'static), start: Address, bytes: usize) {
        debug!("Update SFT for Chunk {} as {}", start, space.name(),);
        let first = start.chunk_index();
        let start_chunk = chunk_index_to_address(first);
        debug!(
            "Update SFT for {} bytes of Chunk {} #{}",
            bytes, start_chunk, first
        );
    }

    fn trace_sft_map(&self) {
        trace!("{}", self.print_sft_map());
    }

    // This can be used during debugging to print SFT map.
    fn print_sft_map(&self) -> String {
        // print the entire SFT map
        let mut res = String::new();

        const SPACE_PER_LINE: usize = 10;
        for i in (0..self.sft.len()).step_by(SPACE_PER_LINE) {
            let max = if i + SPACE_PER_LINE > self.sft.len() {
                self.sft.len()
            } else {
                i + SPACE_PER_LINE
            };
            let chunks: Vec<usize> = (i..max).collect();
            let space_names: Vec<&str> = chunks.iter().map(|&x| self.sft[x].name()).collect();
            res.push_str(&format!(
                "{}: {}",
                chunk_index_to_address(i),
                space_names.join(",")
            ));
            res.push('\n');
        }

        res
    }

    /// Update SFT map for the given address range.
    /// It should be used when we acquire new memory and use it as part of a space. For example, the cases include:
    /// 1. when a space grows, 2. when initializing a contiguous space, 3. when ensure_mapped() is called on a space.
    pub fn update(&self, space: &(dyn SFT + Sync + 'static), start: Address, bytes: usize) {
        if DEBUG_SFT {
            self.log_update(space, start, bytes);
        }
        let first = start.chunk_index();
        let last = conversions::chunk_align_up(start + bytes).chunk_index();
        for chunk in first..last {
            self.set(chunk, space);
        }
        if DEBUG_SFT {
            self.trace_sft_map();
        }
    }

    // TODO: We should clear a SFT entry when a space releases a chunk.
    #[allow(dead_code)]
    pub fn clear(&self, chunk_start: Address) {
        if DEBUG_SFT {
            debug!(
                "Clear SFT for chunk {} (was {})",
                chunk_start,
                self.get(chunk_start).name()
            );
        }
        assert!(chunk_start.is_aligned_to(BYTES_IN_CHUNK));
        let chunk_idx = chunk_start.chunk_index();
        self.set(chunk_idx, &EMPTY_SPACE_SFT);
    }

    // Currently only used by 32 bits vm map
    #[allow(dead_code)]
    pub fn clear_by_index(&self, chunk_idx: usize) {
        if DEBUG_SFT {
            let chunk_start = chunk_index_to_address(chunk_idx);
            debug!(
                "Clear SFT for chunk {} by index (was {})",
                chunk_start,
                self.get(chunk_start).name()
            );
        }
        self.set(chunk_idx, &EMPTY_SPACE_SFT)
    }

    fn set(&self, chunk: usize, sft: &(dyn SFT + Sync + 'static)) {
        /*
         * This is safe (only) because a) this is only called during the
         * allocation and deallocation of chunks, which happens under a global
         * lock, and b) it only transitions from empty to valid and valid to
         * empty, so if there were a race to view the contents, in the one case
         * it would either see the new (valid) space or an empty space (both of
         * which are reasonable), and in the other case it would either see the
         * old (valid) space or an empty space, both of which are valid.
         */
        let self_mut = unsafe { self.mut_self() };
        // It is okay to set empty to valid, or set valid to empty. It is wrong if we overwrite a valid value with another valid value.
        if cfg!(debug_assertions) {
            let old = self_mut.sft[chunk].name();
            let new = sft.name();
            // Allow overwriting the same SFT pointer. E.g., if we have set SFT map for a space, then ensure_mapped() is called on the same,
            // in which case, we still set SFT map again.
            debug_assert!(
                old == EMPTY_SFT_NAME || new == EMPTY_SFT_NAME || old == new,
                "attempt to overwrite a non-empty chunk {} in SFT map (from {} to {})",
                chunk,
                old,
                new
            );
        }
        self_mut.sft[chunk] = sft;
    }

    pub fn is_in_any_space(&self, object: ObjectReference) -> bool {
        if object.to_address().chunk_index() >= self.sft.len() {
            return false;
        }
        self.get(object.to_address()).is_in_space(object)
    }

    #[cfg(feature = "is_mmtk_object")]
    pub fn is_mmtk_object(&self, addr: Address) -> bool {
        if addr.chunk_index() >= self.sft.len() {
            return false;
        }
        self.get(addr).is_mmtk_object(addr)
    }

    /// Make sure we have valid SFT entries for the object reference.
    #[cfg(debug_assertions)]
    pub fn assert_valid_entries_for_object<VM: VMBinding>(&self, object: ObjectReference) {
        use crate::vm::ObjectModel;
        let object_sft = self.get(object.to_address());
        let object_start_sft = self.get(VM::VMObjectModel::object_start_ref(object));

        debug_assert!(
            object_sft.name() != EMPTY_SFT_NAME,
            "Object {} has empty SFT",
            object
        );
        debug_assert_eq!(
            object_sft.name(),
            object_start_sft.name(),
            "Object {} has incorrect SFT entries (object start = {}, object = {}).",
            object,
            object_start_sft.name(),
            object_sft.name()
        );
    }
}
