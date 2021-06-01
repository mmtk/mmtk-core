use super::{
    header_metadata::HeaderMetadata,
    side_metadata::{self, SideMetadata},
};
use crate::util::Address;
use std::{fmt, io::Result};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MetadataScope {
    Global,
    PolicySpecific,
}

impl MetadataScope {
    pub const fn is_global(&self) -> bool {
        matches!(self, MetadataScope::Global)
    }
}

/// This struct stores the specification of a side metadata bit-set.
/// It is used as an input to the (inline) functions provided by the side metadata module.
///
/// Each plan or policy which uses a metadata bit-set, needs to create an instance of this struct.
///
/// For performance reasons, objects of this struct should be constants.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MetadataSpec {
    // true means this is a side metadata
    pub is_on_side: bool,
    pub scope: MetadataScope,
    // for in-header metadata, this is a bit offset,
    // for continuous side metadata, this is a base address,
    // for chunked side metadata, this is the in-chunk offset
    pub offset: usize,
    // for in-header metadata, this can be any number (subject to availability),
    // for side metadata this needs to be a power of 2
    pub num_of_bits: usize,
    // for all metadata, min object size is a power of 2
    pub log_min_obj_size: usize,
}

impl fmt::Debug for MetadataSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "MetadataSpec {{ \
            **is_on_side: {} \
            **Scope: {:?} \
            **offset: 0x{:x} \
            **num_of_bits: 0x{:x} \
            **log_min_obj_size: 0x{:x} \
            }}",
            self.is_on_side, self.scope, self.offset, self.num_of_bits, self.log_min_obj_size
        ))
    }
}

/// This struct stores all the side metadata specs for a policy. Generally a policy needs to know its own
/// side metadata spec as well as the plan's specs.
pub struct MetadataContext {
    // For plans
    pub global: Vec<MetadataSpec>,
    // For policies
    pub local: Vec<MetadataSpec>,
}

impl MetadataContext {
    pub fn new_global_specs(specs: &[MetadataSpec]) -> Vec<MetadataSpec> {
        let mut ret = vec![];
        ret.extend_from_slice(specs);
        if cfg!(feature = "side_gc_header") {
            ret.push(crate::util::gc_byte::SIDE_GC_BYTE_SPEC);
        }
        ret
    }

    pub fn filter(&self, in_header: bool) -> MetadataContext {
        let mut globals = vec![];
        let mut locals = vec![];

        for spec in &self.global {
            if spec.is_on_side ^ in_header {
                globals.push(*spec)
            }
        }

        for spec in &self.local {
            if spec.is_on_side ^ in_header {
                locals.push(*spec)
            }
        }

        MetadataContext {
            global: globals,
            local: locals,
        }
    }
}

pub struct Metadata {
    side_metadata: SideMetadata,
    // do we need this now?
    header_metadata: HeaderMetadata,
    context: MetadataContext,
}

impl Metadata {
    pub fn new(context: MetadataContext) -> Metadata {
        Metadata {
            side_metadata: SideMetadata::new(context.filter(false)),
            header_metadata: HeaderMetadata::new(context.filter(true)),
            context,
        }
    }

    pub fn get_side_metadata_context(&self) -> &MetadataContext {
        self.side_metadata.get_context()
    }

    pub fn get_header_metadata_context(&self) -> &MetadataContext {
        self.header_metadata.get_context()
    }

    pub fn get_local_side_metadata_specs(&self) -> &[MetadataSpec] {
        self.side_metadata.get_local_specs()
    }

    pub fn reserved_pages(&self) -> usize {
        self.side_metadata.reserved_pages()
    }

    pub fn try_map_metadata_space(&self, start: Address, size: usize) -> Result<()> {
        self.side_metadata.try_map_metadata_space(start, size)
    }

    pub fn try_map_metadata_address_range(&self, start: Address, size: usize) -> Result<()> {
        self.side_metadata
            .try_map_metadata_address_range(start, size)
    }

    pub fn ensure_unmap_metadata_space(&self, start: Address, size: usize) {
        self.side_metadata.ensure_unmap_metadata_space(start, size)
    }

    pub fn get_context(&self) -> &MetadataContext {
        &self.context
    }
}

#[inline(always)]
pub fn load_atomic(metadata_spec: MetadataSpec, data_addr: Address) -> usize {
    if metadata_spec.is_on_side {
        side_metadata::load_atomic(metadata_spec, data_addr)
    } else {
        todo!()
    }
}

pub fn store_atomic(metadata_spec: MetadataSpec, data_addr: Address, metadata: usize) {
    if metadata_spec.is_on_side {
        side_metadata::store_atomic(metadata_spec, data_addr, metadata)
    } else {
        todo!()
    }
}

pub fn compare_exchange_atomic(
    metadata_spec: MetadataSpec,
    data_addr: Address,
    old_metadata: usize,
    new_metadata: usize,
) -> bool {
    if metadata_spec.is_on_side {
        side_metadata::compare_exchange_atomic(metadata_spec, data_addr, old_metadata, new_metadata)
    } else {
        todo!()
    }
}

// same as Rust atomics, this wraps around on overflow
pub fn fetch_add_atomic(metadata_spec: MetadataSpec, data_addr: Address, val: usize) -> usize {
    if metadata_spec.is_on_side {
        side_metadata::fetch_add_atomic(metadata_spec, data_addr, val)
    } else {
        todo!()
    }
}

// same as Rust atomics, this wraps around on overflow
pub fn fetch_sub_atomic(metadata_spec: MetadataSpec, data_addr: Address, val: usize) -> usize {
    if metadata_spec.is_on_side {
        side_metadata::fetch_sub_atomic(metadata_spec, data_addr, val)
    } else {
        todo!()
    }
}

/// Non-atomic load of metadata.
///
/// # Safety
///
/// This is unsafe because:
///
/// 1. Concurrent access to this operation is undefined behaviour.
/// 2. Interleaving Non-atomic and atomic operations is undefined behaviour.
///
pub unsafe fn load(metadata_spec: MetadataSpec, data_addr: Address) -> usize {
    if metadata_spec.is_on_side {
        side_metadata::load(metadata_spec, data_addr)
    } else {
        todo!()
    }
}

/// Non-atomic store of metadata.
///
/// # Safety
///
/// This is unsafe because:
///
/// 1. Concurrent access to this operation is undefined behaviour.
/// 2. Interleaving Non-atomic and atomic operations is undefined behaviour.
///
pub unsafe fn store(metadata_spec: MetadataSpec, data_addr: Address, metadata: usize) {
    if metadata_spec.is_on_side {
        side_metadata::store(metadata_spec, data_addr, metadata)
    } else {
        todo!()
    }
}

/// Bulk-zero a specific metadata for a chunk.
///
/// # Arguments
///
/// * `metadata_spec` - The specification of the target side metadata.
///
/// * `chunk_start` - The starting address of the chunk whose metadata is being zeroed.
///
pub fn bzero_metadata(metadata_spec: MetadataSpec, start: Address, size: usize) {
    if metadata_spec.is_on_side {
        side_metadata::bzero_metadata(metadata_spec, start, size)
    } else {
        todo!()
    }
}

#[inline(always)]
pub(crate) fn address_to_side_metadata_address(
    metadata_spec: MetadataSpec,
    data_addr: Address,
) -> Address {
    debug_assert!(
        metadata_spec.is_on_side,
        "address_to_side_metadata_address input MetadataSpec ({:?}) must be on side!",
        metadata_spec
    );
    side_metadata::address_to_meta_address(metadata_spec, data_addr)
}

#[cfg(target_pointer_width = "32")]
#[inline(always)]
pub(crate) const fn side_metadata_bytes_per_chunk(
    log_min_obj_size: usize,
    num_of_bits: usize,
) -> usize {
    side_metadata::meta_bytes_per_chunk(log_min_obj_size, num_of_bits)
}

pub(crate) const fn side_metadata_address_range_size(metadata_spec: MetadataSpec) -> usize {
    side_metadata::metadata_address_range_size(metadata_spec)
}
