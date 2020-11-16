use crate::util::ObjectReference;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use std::sync::atomic::AtomicU8;

/// Return the GC byte of an object.
///
/// MMTk requires *exactly one byte* for each object as per-object metadata.
///
/// If the client VM provides the one byte in its object headers
/// (see [trait ObjectModel](crate::vm::ObjectModel)),
/// MMTk uses that byte as the per-object metadata.
/// Otherwise, MMTk provides the metadata on its side.
///
pub fn get_gc_byte<VM: VMBinding>(object: ObjectReference) -> &'static AtomicU8 {
    if VM::VMObjectModel::HAS_GC_BYTE {
        unsafe {
            &*(object.to_address() + VM::VMObjectModel::GC_BYTE_OFFSET / 8).to_ptr::<AtomicU8>()
        }
    } else {
        todo!("\"HAS_GC_BYTE == false\" is not supported yet")
    }
}

/// Return the offset of a GC byte relative to its containing header word.
///
/// For cases where the constant `GC_BYTE_OFFSET` is negative (e.g. JikesRVM),
/// this function returns a positive offset
/// value in the [0 to word size) range.
///
pub fn get_relative_offset<VM: VMBinding>() -> isize {
    #[cfg(target_pointer_width = "64")]
    let sys_ptr_width = 64;
    #[cfg(target_pointer_width = "32")]
    let sys_ptr_width = 32;
    (VM::VMObjectModel::GC_BYTE_OFFSET).rem_euclid(sys_ptr_width)
}
