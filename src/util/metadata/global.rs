use crate::util::metadata::side_metadata;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::ObjectReference;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use atomic::Ordering;

use super::header_metadata::HeaderMetadataSpec;

/// This struct stores the specification of a metadata bit-set.
/// It is used as an input to the (inline) functions provided by the side metadata module.
///
/// Each plan or policy which uses a metadata bit-set, needs to create an instance of this struct.
///
/// For performance reasons, objects of this struct should be constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetadataSpec {
    InHeader(HeaderMetadataSpec),
    OnSide(SideMetadataSpec),
}

impl MetadataSpec {
    pub fn is_on_side(&self) -> bool {
        matches!(self, &MetadataSpec::OnSide(_))
    }
    pub const fn as_header(self) -> Option<HeaderMetadataSpec> {
        match self{
            Self::InHeader(h) => Some(h),
            _ => None
        }
    }
    pub const fn as_side(self) -> Option<SideMetadataSpec> {
        match self{
            Self::OnSide(s) => Some(s),
            _ => None
        }
    }
}

/// A function to load the specified metadata's content.
///
/// # Arguments:
///
/// * `metadata_spec`: is one of the const `MetadataSpec` instances from the ObjectModel trait, for the target metadata. Whether the metadata is in-header or on-side is a VM-specific choice.
/// * `object`: is a reference to the target object.
/// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
/// * `atomic_ordering`: is an optional atomic ordering for the load operation. An input value of `None` means the load operation is not atomic, and an input value of `Some(Ordering::X)` means the atomic load operation will use the `Ordering::X`.
///
/// # Returns the metadata value as a word. If the metadata size is less than a word, the effective value is stored in the low-order bits of the word.
///
#[inline(always)]
pub fn load_metadata<VM: VMBinding>(
    metadata_spec: &MetadataSpec,
    object: ObjectReference,
    mask: Option<usize>,
    atomic_ordering: Option<Ordering>,
) -> usize {
    match metadata_spec {
        MetadataSpec::OnSide(metadata_spec) => {
            if let Some(order) = atomic_ordering {
                side_metadata::load_atomic(metadata_spec, object.to_address(), order)
            } else {
                unsafe { side_metadata::load(metadata_spec, object.to_address()) }
            }
        }
        MetadataSpec::InHeader(metadata_spec) => {
            VM::VMObjectModel::load_metadata(metadata_spec, object, mask, atomic_ordering)
        }
    }
}

/// A function to store a value to the specified metadata.
///
/// # Arguments:
///
/// * `metadata_spec`: is one of the const `MetadataSpec` instances from the ObjectModel trait, for the target metadata. Whether the metadata is in-header or on-side is a VM-specific choice.
/// * `object`: is a reference to the target object.
/// * `val`: is the new metadata value to be stored.
/// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
/// * `atomic_ordering`: is an optional atomic ordering for the store operation. An input value of `None` means the store operation is not atomic, and an input value of `Some(Ordering::X)` means the atomic store operation will use the `Ordering::X`.
///
#[inline(always)]
pub fn store_metadata<VM: VMBinding>(
    metadata_spec: &MetadataSpec,
    object: ObjectReference,
    val: usize,
    mask: Option<usize>,
    atomic_ordering: Option<Ordering>,
) {
    match metadata_spec {
        MetadataSpec::OnSide(metadata_spec) => {
            if let Some(order) = atomic_ordering {
                side_metadata::store_atomic(metadata_spec, object.to_address(), val, order);
            } else {
                unsafe {
                    side_metadata::store(metadata_spec, object.to_address(), val);
                }
            }
        }
        MetadataSpec::InHeader(metadata_spec) => {
            VM::VMObjectModel::store_metadata(metadata_spec, object, val, mask, atomic_ordering);
        }
    }
}

/// A function to atomically compare-and-exchange the specified metadata's content.
///
/// # Arguments:
///
/// * `metadata_spec`: is one of the const `MetadataSpec` instances from the ObjectModel trait, for the target metadata. Whether the metadata is in-header or on-side is a VM-specific choice.
/// * `object`: is a reference to the target object.
/// * `old_val`: is the expected current value of the metadata.
/// * `new_val`: is the new metadata value to be stored if the compare-and-exchange operation is successful.
/// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
/// * `success_order`: is the atomic ordering used if the operation is successful.
/// * `failure_order`: is the atomic ordering used if the operation fails.
///
/// # Returns `true` if the operation is successful, and `false` otherwise.
///
#[inline(always)]
pub fn compare_exchange_metadata<VM: VMBinding>(
    metadata_spec: &MetadataSpec,
    object: ObjectReference,
    old_val: usize,
    new_val: usize,
    mask: Option<usize>,
    success_order: Ordering,
    failure_order: Ordering,
) -> bool {
    match metadata_spec {
        MetadataSpec::OnSide(metadata_spec) => side_metadata::compare_exchange_atomic(
            metadata_spec,
            object.to_address(),
            old_val,
            new_val,
            success_order,
            failure_order,
        ),
        MetadataSpec::InHeader(metadata_spec) => VM::VMObjectModel::compare_exchange_metadata(
            metadata_spec,
            object,
            old_val,
            new_val,
            mask,
            success_order,
            failure_order,
        ),
    }
}

/// A function to atomically perform an add operation on the specified metadata's content.
///
/// # Arguments:
///
/// * `metadata_spec`: is one of the const `MetadataSpec` instances from the ObjectModel trait, for the target metadata. Whether the metadata is in-header or on-side is a VM-specific choice.
/// * `object`: is a reference to the target object.
/// * `val`: is the value to be added to the current value of the metadata.
/// * `order`: is the atomic ordering of the fetch-and-add operation.
///
/// # Returns the old metadata value as a word.
///
#[inline(always)]
pub fn fetch_add_metadata<VM: VMBinding>(
    metadata_spec: &MetadataSpec,
    object: ObjectReference,
    val: usize,
    order: Ordering,
) -> usize {
    match metadata_spec {
        MetadataSpec::OnSide(metadata_spec) => {
            side_metadata::fetch_add_atomic(metadata_spec, object.to_address(), val, order)
        }
        MetadataSpec::InHeader(metadata_spec) => {
            VM::VMObjectModel::fetch_add_metadata(metadata_spec, object, val, order)
        }
    }
}

/// A function to atomically perform a subtract operation on the specified metadata's content.
///
/// # Arguments:
///
/// * `metadata_spec`: is one of the const `MetadataSpec` instances from the ObjectModel trait, for the target metadata. Whether the metadata is in-header or on-side is a VM-specific choice.
/// * `object`: is a reference to the target object.
/// * `val`: is the value to be subtracted from the current value of the metadata.
/// * `order`: is the atomic ordering of the fetch-and-add operation.
///
/// # Returns the old metadata value as a word.
///
#[inline(always)]
pub fn fetch_sub_metadata<VM: VMBinding>(
    metadata_spec: &MetadataSpec,
    object: ObjectReference,
    val: usize,
    order: Ordering,
) -> usize {
    match metadata_spec {
        MetadataSpec::OnSide(metadata_spec) => {
            side_metadata::fetch_sub_atomic(metadata_spec, object.to_address(), val, order)
        }
        MetadataSpec::InHeader(metadata_spec) => {
            VM::VMObjectModel::fetch_sub_metadata(metadata_spec, object, val, order)
        }
    }
}

/// Given a slice of metadata specifications, returns a vector of the specs which are on side.
///
/// # Arguments:
/// * `specs` is the input slice of on-side and/or in-header metadata specifications.
///
pub(crate) fn extract_side_metadata(specs: &[MetadataSpec]) -> Vec<SideMetadataSpec> {
    let mut side_specs = vec![];
    for spec in specs {
        if let MetadataSpec::OnSide(ss) = *spec {
            side_specs.push(ss);
        }
    }

    side_specs
}
