use crate::util::constants::*;
use crate::util::heap::layout::vm_layout::*;
use crate::util::linear_scan::Region;
use crate::util::metadata::side_metadata::constants::{
    GLOBAL_SIDE_METADATA_BASE_OFFSET, LOCAL_SIDE_METADATA_BASE_OFFSET,
};
use crate::util::metadata::side_metadata::SideMetadataOffset;
use crate::util::metadata::side_metadata::SideMetadataSpec;

// This macro helps define side metadata specs, and layout their offsets one after another.
// The macro is implemented with the incremental TT muncher pattern (see https://danielkeep.github.io/tlborm/book/pat-incremental-tt-munchers.html).
// This should only be used twice within mmtk-core: one for global specs, and one for local specs.
// This should not be used to layout VM specs (we have provided side_first()/side_after() for the VM side metadata specs).
macro_rules! define_side_metadata_specs {
    // Internal patterns

    // Define the first spec with offset at either GLOBAL/LOCAL_SIDE_METADATA_BASE_OFFSET
    (@first_spec $name: ident = (global: $is_global: expr, log_num_of_bits: $log_num_of_bits: expr, log_bytes_in_region: $log_bytes_in_region: expr)) => {
        pub const $name: SideMetadataSpec = SideMetadataSpec {
            name: stringify!($name),
            is_global: $is_global,
            offset: if $is_global { GLOBAL_SIDE_METADATA_BASE_OFFSET } else { LOCAL_SIDE_METADATA_BASE_OFFSET },
            log_num_of_bits: $log_num_of_bits,
            log_bytes_in_region: $log_bytes_in_region,
        };
    };
    // Define any spec that follows a previous spec. The new spec will be created and laid out after the previous spec.
    (@prev_spec $last_spec: ident as $last_spec_ident: ident, $name: ident = (global: $is_global: expr, log_num_of_bits: $log_num_of_bits: expr, log_bytes_in_region: $log_bytes_in_region: expr), $($tail:tt)*) => {
        pub const $name: SideMetadataSpec = SideMetadataSpec {
            name: stringify!($name),
            is_global: $is_global,
            offset: SideMetadataOffset::layout_after(&$last_spec),
            log_num_of_bits: $log_num_of_bits,
            log_bytes_in_region: $log_bytes_in_region,
        };
        define_side_metadata_specs!(@prev_spec $name as $last_spec_ident, $($tail)*);
    };
    // Define the last spec with the given identifier.
    (@prev_spec $last_spec: ident as $last_spec_ident: ident,) => {
        pub const $last_spec_ident: SideMetadataSpec = $last_spec;
    };

    // The actual macro

    // This is the pattern that should be used outside this macro.
    (last_spec_as $last_spec_ident: ident, $name0: ident = (global: $is_global0: expr, log_num_of_bits: $log_num_of_bits0: expr, log_bytes_in_region: $log_bytes_in_region0: expr), $($tail:tt)*) => {
        // Defines the first spec
        define_side_metadata_specs!(@first_spec $name0 = (global: $is_global0, log_num_of_bits: $log_num_of_bits0, log_bytes_in_region: $log_bytes_in_region0));
        // The rest specs
        define_side_metadata_specs!(@prev_spec $name0 as $last_spec_ident, $($tail)*);
    };
}

// This defines all GLOBAL side metadata used by mmtk-core.
define_side_metadata_specs!(
    last_spec_as LAST_GLOBAL_SIDE_METADATA_SPEC,
    // Mark the start of an object
    VO_BIT       = (global: true, log_num_of_bits: 0, log_bytes_in_region: LOG_MIN_OBJECT_SIZE as usize),
    // Track chunks used by (malloc) marksweep
    MS_ACTIVE_CHUNK = (global: true, log_num_of_bits: 3, log_bytes_in_region: LOG_BYTES_IN_CHUNK),
    // Track the index in SFT map for a chunk (only used for SFT sparse chunk map)
    SFT_DENSE_CHUNK_MAP_INDEX   = (global: true, log_num_of_bits: 3, log_bytes_in_region: LOG_BYTES_IN_CHUNK),
);

// This defines all LOCAL side metadata used by mmtk-core.
define_side_metadata_specs!(
    last_spec_as LAST_LOCAL_SIDE_METADATA_SPEC,
    // Mark pages by (malloc) marksweep
    MALLOC_MS_ACTIVE_PAGE  = (global: false, log_num_of_bits: 3, log_bytes_in_region: crate::util::malloc::library::LOG_BYTES_IN_MALLOC_PAGE as usize),
    // Record objects allocated with some offset
    MS_OFFSET_MALLOC = (global: false, log_num_of_bits: 0, log_bytes_in_region: LOG_MIN_OBJECT_SIZE as usize),
    // Mark lines by immix
    IX_LINE_MARK    = (global: false, log_num_of_bits: 3, log_bytes_in_region: crate::policy::immix::line::Line::LOG_BYTES),
    // Record defrag state for immix blocks
    IX_BLOCK_DEFRAG = (global: false, log_num_of_bits: 3, log_bytes_in_region: crate::policy::immix::block::Block::LOG_BYTES),
    // Mark blocks by immix
    IX_BLOCK_MARK   = (global: false, log_num_of_bits: 3, log_bytes_in_region: crate::policy::immix::block::Block::LOG_BYTES),
    // Mark chunks (any plan that uses the chunk map should include this spec in their local sidemetadata specs)
    CHUNK_MARK   = (global: false, log_num_of_bits: 3, log_bytes_in_region: crate::util::heap::chunk_map::Chunk::LOG_BYTES),
    // Mark blocks by (native mimalloc) marksweep
    MS_BLOCK_MARK   = (global: false, log_num_of_bits: 3, log_bytes_in_region: crate::policy::marksweepspace::native_ms::Block::LOG_BYTES),
    // Next block in list for native mimalloc
    MS_BLOCK_NEXT   = (global: false, log_num_of_bits: LOG_BITS_IN_ADDRESS, log_bytes_in_region: crate::policy::marksweepspace::native_ms::Block::LOG_BYTES),
    // Previous block in list for native mimalloc
    MS_BLOCK_PREV   = (global: false, log_num_of_bits: LOG_BITS_IN_ADDRESS, log_bytes_in_region: crate::policy::marksweepspace::native_ms::Block::LOG_BYTES),
    // Pointer to owning list for blocks for native mimalloc
    MS_BLOCK_LIST   = (global: false, log_num_of_bits: LOG_BITS_IN_ADDRESS, log_bytes_in_region: crate::policy::marksweepspace::native_ms::Block::LOG_BYTES),
    // Size of cells in block for native mimalloc FIXME: do we actually need usize?
    MS_BLOCK_SIZE         = (global: false, log_num_of_bits: LOG_BITS_IN_ADDRESS, log_bytes_in_region: crate::policy::marksweepspace::native_ms::Block::LOG_BYTES),
    // TLS of owning mutator of block for native mimalloc
    MS_BLOCK_TLS    = (global: false, log_num_of_bits: LOG_BITS_IN_ADDRESS, log_bytes_in_region: crate::policy::marksweepspace::native_ms::Block::LOG_BYTES),
    // First cell of free list in block for native mimalloc
    MS_FREE         = (global: false, log_num_of_bits: LOG_BITS_IN_ADDRESS, log_bytes_in_region: crate::policy::marksweepspace::native_ms::Block::LOG_BYTES),
    // The following specs are only used for manual malloc/free
    // First cell of local free list in block for native mimalloc
    MS_LOCAL_FREE   = (global: false, log_num_of_bits: LOG_BITS_IN_ADDRESS, log_bytes_in_region: crate::policy::marksweepspace::native_ms::Block::LOG_BYTES),
    // First cell of thread free list in block for native mimalloc
    MS_THREAD_FREE  = (global: false, log_num_of_bits: LOG_BITS_IN_ADDRESS, log_bytes_in_region: crate::policy::marksweepspace::native_ms::Block::LOG_BYTES),
);

#[cfg(test)]
mod tests {
    // We assert on constants to test if the macro is working properly.
    #![allow(clippy::assertions_on_constants)]

    use super::*;
    #[test]
    fn first_global_spec() {
        define_side_metadata_specs!(last_spec_as LAST_GLOBAL_SPEC, TEST_SPEC = (global: true, log_num_of_bits: 0, log_bytes_in_region: 3),);
        assert!(TEST_SPEC.is_global);
        assert!(TEST_SPEC.offset == GLOBAL_SIDE_METADATA_BASE_OFFSET);
        assert_eq!(TEST_SPEC.log_num_of_bits, 0);
        assert_eq!(TEST_SPEC.log_bytes_in_region, 3);
        assert_eq!(TEST_SPEC, LAST_GLOBAL_SPEC);
    }

    #[test]
    fn first_local_spec() {
        define_side_metadata_specs!(last_spec_as LAST_LOCAL_SPEC, TEST_SPEC = (global: false, log_num_of_bits: 0, log_bytes_in_region: 3),);
        assert!(!TEST_SPEC.is_global);
        assert!(TEST_SPEC.offset == LOCAL_SIDE_METADATA_BASE_OFFSET);
        assert_eq!(TEST_SPEC.log_num_of_bits, 0);
        assert_eq!(TEST_SPEC.log_bytes_in_region, 3);
        assert_eq!(TEST_SPEC, LAST_LOCAL_SPEC);
    }

    #[test]
    fn two_global_specs() {
        define_side_metadata_specs!(
            last_spec_as LAST_GLOBAL_SPEC,
            TEST_SPEC1 = (global: true, log_num_of_bits: 0, log_bytes_in_region: 3),
            TEST_SPEC2 = (global: true, log_num_of_bits: 1, log_bytes_in_region: 4),
        );

        assert!(TEST_SPEC1.is_global);
        assert!(TEST_SPEC1.offset == GLOBAL_SIDE_METADATA_BASE_OFFSET);
        assert_eq!(TEST_SPEC1.log_num_of_bits, 0);
        assert_eq!(TEST_SPEC1.log_bytes_in_region, 3);

        assert!(TEST_SPEC2.is_global);
        assert!(TEST_SPEC2.offset == SideMetadataOffset::layout_after(&TEST_SPEC1));
        assert_eq!(TEST_SPEC2.log_num_of_bits, 1);
        assert_eq!(TEST_SPEC2.log_bytes_in_region, 4);

        assert_eq!(TEST_SPEC2, LAST_GLOBAL_SPEC);
    }

    #[test]
    fn three_global_specs() {
        define_side_metadata_specs!(
            last_spec_as LAST_GLOBAL_SPEC,
            TEST_SPEC1 = (global: true, log_num_of_bits: 0, log_bytes_in_region: 3),
            TEST_SPEC2 = (global: true, log_num_of_bits: 1, log_bytes_in_region: 4),
            TEST_SPEC3 = (global: true, log_num_of_bits: 2, log_bytes_in_region: 5),
        );

        assert!(TEST_SPEC1.is_global);
        assert!(TEST_SPEC1.offset == GLOBAL_SIDE_METADATA_BASE_OFFSET);
        assert_eq!(TEST_SPEC1.log_num_of_bits, 0);
        assert_eq!(TEST_SPEC1.log_bytes_in_region, 3);

        assert!(TEST_SPEC2.is_global);
        assert!(TEST_SPEC2.offset == SideMetadataOffset::layout_after(&TEST_SPEC1));
        assert_eq!(TEST_SPEC2.log_num_of_bits, 1);
        assert_eq!(TEST_SPEC2.log_bytes_in_region, 4);

        assert!(TEST_SPEC3.is_global);
        assert!(TEST_SPEC3.offset == SideMetadataOffset::layout_after(&TEST_SPEC2));
        assert_eq!(TEST_SPEC3.log_num_of_bits, 2);
        assert_eq!(TEST_SPEC3.log_bytes_in_region, 5);

        assert_eq!(TEST_SPEC3, LAST_GLOBAL_SPEC);
    }

    #[test]
    fn both_global_and_local() {
        define_side_metadata_specs!(
            last_spec_as LAST_GLOBAL_SPEC,
            TEST_GSPEC1 = (global: true, log_num_of_bits: 0, log_bytes_in_region: 3),
            TEST_GSPEC2 = (global: true, log_num_of_bits: 1, log_bytes_in_region: 4),
        );
        define_side_metadata_specs!(
            last_spec_as LAST_LOCAL_SPEC,
            TEST_LSPEC1 = (global: false, log_num_of_bits: 2, log_bytes_in_region: 5),
            TEST_LSPEC2 = (global: false, log_num_of_bits: 3, log_bytes_in_region: 6),
        );

        assert!(TEST_GSPEC1.is_global);
        assert!(TEST_GSPEC1.offset == GLOBAL_SIDE_METADATA_BASE_OFFSET);
        assert_eq!(TEST_GSPEC1.log_num_of_bits, 0);
        assert_eq!(TEST_GSPEC1.log_bytes_in_region, 3);

        assert!(TEST_GSPEC2.is_global);
        assert!(TEST_GSPEC2.offset == SideMetadataOffset::layout_after(&TEST_GSPEC1));
        assert_eq!(TEST_GSPEC2.log_num_of_bits, 1);
        assert_eq!(TEST_GSPEC2.log_bytes_in_region, 4);

        assert_eq!(TEST_GSPEC2, LAST_GLOBAL_SPEC);

        assert!(!TEST_LSPEC1.is_global);
        assert!(TEST_LSPEC1.offset == LOCAL_SIDE_METADATA_BASE_OFFSET);
        assert_eq!(TEST_LSPEC1.log_num_of_bits, 2);
        assert_eq!(TEST_LSPEC1.log_bytes_in_region, 5);

        assert!(!TEST_LSPEC2.is_global);
        assert!(TEST_LSPEC2.offset == SideMetadataOffset::layout_after(&TEST_LSPEC1));
        assert_eq!(TEST_LSPEC2.log_num_of_bits, 3);
        assert_eq!(TEST_LSPEC2.log_bytes_in_region, 6);

        assert_eq!(TEST_LSPEC2, LAST_LOCAL_SPEC);
    }
}
