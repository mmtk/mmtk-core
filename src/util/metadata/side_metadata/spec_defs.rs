use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::metadata::side_metadata::constants::{GLOBAL_SIDE_METADATA_BASE_OFFSET, LOCAL_SIDE_METADATA_BASE_OFFSET};
use crate::util::metadata::side_metadata::SideMetadataOffset;
use crate::util::constants::*;
use crate::util::heap::layout::vm_layout_constants::*;

macro_rules! define_side_metadata_specs {
    (@prev_spec $last_spec: ident as $last_spec_ident: ident, $name: ident = (global: $is_global: expr, log_num_of_bits: $log_num_of_bits: expr, log_bytes_in_region: $log_bytes_in_region: expr), $($tail:tt)*) => {
        pub const $name: SideMetadataSpec = SideMetadataSpec {
            is_global: $is_global,
            offset: SideMetadataOffset::layout_after(&$last_spec),
            log_num_of_bits: $log_num_of_bits,
            log_bytes_in_region: $log_bytes_in_region,
        };
        define_side_metadata_specs!(@prev_spec $name as $last_spec_ident, $($tail)*);
    };
    (@prev_spec $last_spec: ident as $last_spec_ident: ident,) => {
        pub const $last_spec_ident: SideMetadataSpec = $last_spec;
    };
    (@first_spec $name: ident = (global: $is_global: expr, log_num_of_bits: $log_num_of_bits: expr, log_bytes_in_region: $log_bytes_in_region: expr)) => {
        pub const $name: SideMetadataSpec = SideMetadataSpec {
            is_global: $is_global,
            offset: if $is_global { GLOBAL_SIDE_METADATA_BASE_OFFSET } else { LOCAL_SIDE_METADATA_BASE_OFFSET },
            log_num_of_bits: $log_num_of_bits,
            log_bytes_in_region: $log_bytes_in_region,
        };
    };
    (last_spec_as $last_spec_ident: ident, $name0: ident = (global: $is_global0: expr, log_num_of_bits: $log_num_of_bits0: expr, log_bytes_in_region: $log_bytes_in_region0: expr), $($tail:tt)*) => {
        define_side_metadata_specs!(@first_spec $name0 = (global: $is_global0, log_num_of_bits: $log_num_of_bits0, log_bytes_in_region: $log_bytes_in_region0));
        define_side_metadata_specs!(@prev_spec $name0 as $last_spec_ident, $($tail)*);
    };
}

// This defines all GLOBAL side metadata used by mmtk-core.
define_side_metadata_specs!(
    last_spec_as LAST_GLOBAL_SIDE_METADATA_SPEC,
    // Mark the start of an object
    ALLOC_BIT       = (global: true, log_num_of_bits: 0, log_bytes_in_region: LOG_MIN_OBJECT_SIZE as usize),
    // Track chunks used by (malloc) marksweep
    MS_ACTIVE_CHUNK = (global: true, log_num_of_bits: 3, log_bytes_in_region: LOG_BYTES_IN_CHUNK as usize),
);

// This defines all LOCAL side metadata used by mmtk-core.
define_side_metadata_specs!(
    last_spec_as LAST_LOCAL_SIDE_METADATA_SPEC,
    // Mark pages by (malloc) marksweep
    MS_ACTIVE_PAGE  = (global: false, log_num_of_bits: 3, log_bytes_in_region: LOG_BYTES_IN_PAGE as usize),
    // Mark lines by immix
    IX_LINE_MARK    = (global: false, log_num_of_bits: 3, log_bytes_in_region: crate::policy::immix::line::Line::LOG_BYTES),
    // Record defrag state for immix blocks
    IX_BLOCK_DEFRAG = (global: false, log_num_of_bits: 3, log_bytes_in_region: crate::policy::immix::block::Block::LOG_BYTES),
    // Mark blocks by immix;
    IX_BLOCK_MARK   = (global: false, log_num_of_bits: 3, log_bytes_in_region: crate::policy::immix::block::Block::LOG_BYTES),
    IX_CHUNK_MARK   = (global: false, log_num_of_bits: 3, log_bytes_in_region: crate::policy::immix::chunk::Chunk::LOG_BYTES),
);

#[cfg(test)]
mod tests {
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