use super::header_metadata::HeaderMetadataSpec;
use crate::util::metadata::metadata_val_traits::*;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::ObjectReference;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use atomic::Ordering;

/// This struct stores the specification of a metadata bit-set.
/// It is used as an input to the (inline) functions provided by the side metadata module.
///
/// Each plan or policy which uses a metadata bit-set, needs to create an instance of this struct.
///
/// For performance reasons, objects of this struct should be constants.
#[derive(Clone, Copy, Debug)]
pub enum MetadataSpec {
    /// In-header metadata uses bits from an object header.
    InHeader(HeaderMetadataSpec),
    /// On-side metadata uses a side table.
    OnSide(SideMetadataSpec),
}

impl MetadataSpec {
    /// Is this metadata stored in the side table?
    pub const fn is_on_side(&self) -> bool {
        matches!(self, &MetadataSpec::OnSide(_))
    }

    /// Is this metadata stored in the object header?
    pub const fn is_in_header(&self) -> bool {
        matches!(self, &MetadataSpec::InHeader(_))
    }

    /// Extract SideMetadataSpec from a MetadataSpec. Panics if this is not side metadata.
    pub const fn extract_side_spec(&self) -> &SideMetadataSpec {
        match self {
            MetadataSpec::OnSide(spec) => spec,
            MetadataSpec::InHeader(_) => panic!("Expect a side spec"),
        }
    }

    /// A function to non-atomically load the specified metadata's content.
    /// Returns the metadata value.
    ///
    /// # Arguments:
    ///
    /// * `object`: is a reference to the target object.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    ///
    /// # Safety
    /// This is a non-atomic load, thus not thread-safe.
    pub unsafe fn load<VM: VMBinding, T: MetadataValue>(
        &self,
        object: ObjectReference,
        mask: Option<T>,
    ) -> T {
        match self {
            MetadataSpec::OnSide(metadata_spec) => metadata_spec.load(object.to_address::<VM>()),
            MetadataSpec::InHeader(metadata_spec) => {
                VM::VMObjectModel::load_metadata::<T>(metadata_spec, object, mask)
            }
        }
    }

    /// A function to atomically load the specified metadata's content.
    /// Returns the metadata value.
    ///
    /// # Arguments:
    ///
    /// * `object`: is a reference to the target object.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `atomic_ordering`: is the ordering for the load operation.
    ///
    pub fn load_atomic<VM: VMBinding, T: MetadataValue>(
        &self,
        object: ObjectReference,
        mask: Option<T>,
        ordering: Ordering,
    ) -> T {
        match self {
            MetadataSpec::OnSide(metadata_spec) => {
                metadata_spec.load_atomic(object.to_address::<VM>(), ordering)
            }
            MetadataSpec::InHeader(metadata_spec) => {
                VM::VMObjectModel::load_metadata_atomic::<T>(metadata_spec, object, mask, ordering)
            }
        }
    }

    /// A function to non-atomically store a value to the specified metadata.
    ///
    /// # Arguments:
    ///
    /// * `object`: is a reference to the target object.
    /// * `val`: is the new metadata value to be stored.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    ///
    /// # Safety
    /// This is a non-atomic store, thus not thread-safe.
    pub unsafe fn store<VM: VMBinding, T: MetadataValue>(
        &self,
        object: ObjectReference,
        val: T,
        mask: Option<T>,
    ) {
        match self {
            MetadataSpec::OnSide(metadata_spec) => {
                metadata_spec.store(object.to_address::<VM>(), val);
            }
            MetadataSpec::InHeader(metadata_spec) => {
                VM::VMObjectModel::store_metadata::<T>(metadata_spec, object, val, mask)
            }
        }
    }

    /// A function to atomically store a value to the specified metadata.
    ///
    /// # Arguments:
    ///
    /// * `object`: is a reference to the target object.
    /// * `val`: is the new metadata value to be stored.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `ordering`: is the ordering for the store operation.
    pub fn store_atomic<VM: VMBinding, T: MetadataValue>(
        &self,
        object: ObjectReference,
        val: T,
        mask: Option<T>,
        ordering: Ordering,
    ) {
        match self {
            MetadataSpec::OnSide(metadata_spec) => {
                metadata_spec.store_atomic(object.to_address::<VM>(), val, ordering);
            }
            MetadataSpec::InHeader(metadata_spec) => VM::VMObjectModel::store_metadata_atomic::<T>(
                metadata_spec,
                object,
                val,
                mask,
                ordering,
            ),
        }
    }

    /// A function to atomically compare-and-exchange the specified metadata's content.
    /// Returns `true` if the operation is successful, and `false` otherwise.
    ///
    /// # Arguments:
    ///
    /// * `object`: is a reference to the target object.
    /// * `old_val`: is the expected current value of the metadata.
    /// * `new_val`: is the new metadata value to be stored if the compare-and-exchange operation is successful.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `success_order`: is the atomic ordering used if the operation is successful.
    /// * `failure_order`: is the atomic ordering used if the operation fails.
    ///
    pub fn compare_exchange_metadata<VM: VMBinding, T: MetadataValue>(
        &self,
        object: ObjectReference,
        old_val: T,
        new_val: T,
        mask: Option<T>,
        success_order: Ordering,
        failure_order: Ordering,
    ) -> std::result::Result<T, T> {
        match self {
            MetadataSpec::OnSide(metadata_spec) => metadata_spec.compare_exchange_atomic(
                object.to_address::<VM>(),
                old_val,
                new_val,
                success_order,
                failure_order,
            ),
            MetadataSpec::InHeader(metadata_spec) => {
                VM::VMObjectModel::compare_exchange_metadata::<T>(
                    metadata_spec,
                    object,
                    old_val,
                    new_val,
                    mask,
                    success_order,
                    failure_order,
                )
            }
        }
    }

    /// A function to atomically perform an add operation on the specified metadata's content.
    /// Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to be added to the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    ///
    pub fn fetch_add_metadata<VM: VMBinding, T: MetadataValue>(
        &self,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        match self {
            MetadataSpec::OnSide(metadata_spec) => {
                metadata_spec.fetch_add_atomic(object.to_address::<VM>(), val, order)
            }
            MetadataSpec::InHeader(metadata_spec) => {
                VM::VMObjectModel::fetch_add_metadata::<T>(metadata_spec, object, val, order)
            }
        }
    }

    /// A function to atomically perform a subtract operation on the specified metadata's content.
    /// Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to be subtracted from the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    ///
    pub fn fetch_sub_metadata<VM: VMBinding, T: MetadataValue>(
        &self,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        match self {
            MetadataSpec::OnSide(metadata_spec) => {
                metadata_spec.fetch_sub_atomic(object.to_address::<VM>(), val, order)
            }
            MetadataSpec::InHeader(metadata_spec) => {
                VM::VMObjectModel::fetch_sub_metadata::<T>(metadata_spec, object, val, order)
            }
        }
    }

    /// A function to atomically perform a bit-and operation on the specified metadata's content.
    /// Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to bit-and with the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    ///
    pub fn fetch_and_metadata<VM: VMBinding, T: MetadataValue>(
        &self,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        match self {
            MetadataSpec::OnSide(metadata_spec) => {
                metadata_spec.fetch_and_atomic(object.to_address::<VM>(), val, order)
            }
            MetadataSpec::InHeader(metadata_spec) => {
                VM::VMObjectModel::fetch_and_metadata::<T>(metadata_spec, object, val, order)
            }
        }
    }

    /// A function to atomically perform a bit-or operation on the specified metadata's content.
    /// Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to bit-or with the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    ///
    pub fn fetch_or_metadata<VM: VMBinding, T: MetadataValue>(
        &self,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        match self {
            MetadataSpec::OnSide(metadata_spec) => {
                metadata_spec.fetch_or_atomic(object.to_address::<VM>(), val, order)
            }
            MetadataSpec::InHeader(metadata_spec) => {
                VM::VMObjectModel::fetch_or_metadata::<T>(metadata_spec, object, val, order)
            }
        }
    }

    /// A function to atomically perform an update operation on the specified metadata's content. The semantics are the same as Rust's `fetch_update` on atomic types.
    /// Returns a Result of Ok(previous_value) if the function returned Some(_), else Err(previous_value).
    ///
    /// # Arguments:
    ///
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to bit-or with the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    /// * `f`: the update function. The function may be called multiple times.
    ///
    pub fn fetch_update_metadata<
        VM: VMBinding,
        T: MetadataValue,
        F: FnMut(T) -> Option<T> + Copy,
    >(
        &self,
        object: ObjectReference,
        set_order: Ordering,
        fetch_order: Ordering,
        f: F,
    ) -> std::result::Result<T, T> {
        match self {
            MetadataSpec::OnSide(metadata_spec) => metadata_spec.fetch_update_atomic(
                object.to_address::<VM>(),
                set_order,
                fetch_order,
                f,
            ),
            MetadataSpec::InHeader(metadata_spec) => VM::VMObjectModel::fetch_update_metadata(
                metadata_spec,
                object,
                set_order,
                fetch_order,
                f,
            ),
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
