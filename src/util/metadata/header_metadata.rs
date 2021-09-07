use std::fmt;
use std::sync::atomic::AtomicU16;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::AtomicUsize;

use atomic::Ordering;

use crate::util::constants::BITS_IN_BYTE;
use crate::util::constants::BITS_IN_WORD;
use crate::util::constants::LOG_BITS_IN_BYTE;
use crate::util::constants::LOG_BITS_IN_WORD;
use crate::util::ObjectReference;

const LOG_BITS_IN_U16: usize = 4;
const BIT_IN_U16: usize = 1 << LOG_BITS_IN_U16;
const LOG_BITS_IN_U32: usize = 5;
const BITS_IN_U32: usize = 1 << LOG_BITS_IN_U32;

/// This module provides a default implementation of the access functions for in-header metadata.

/// This struct stores the specification of a header metadata bit-set.
/// It is used as an input to the (inline) functions provided by the header metadata module or ObjectModel metadata modules.
///
/// For performance reasons, objects of this struct should be constants.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeaderMetadataSpec {
    pub bit_offset: isize,
    pub num_of_bits: usize,
}

impl fmt::Debug for HeaderMetadataSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "HeaderMetadataSpec {{ \
            **bit_offset: 0x{:x} \
            **num_of_bits: 0x{:x} \
            }}",
            self.bit_offset, self.num_of_bits
        ))
    }
}

/// This function provides a default implementation for the `load_metadata` method from the `ObjectModel` trait.
#[inline(always)]
pub fn load_metadata(
    metadata_spec: &HeaderMetadataSpec,
    object: ObjectReference,
    optional_mask: Option<usize>,
    atomic_ordering: Option<Ordering>,
) -> usize {
    debug_assert!(optional_mask.is_none() || metadata_spec.num_of_bits >= 8,"optional_mask is only supported for 8X-bits in-header metadata. Problematic MetadataSpec: ({:?})", metadata_spec);

    // metadata smaller than 8-bits is special in that more than one metadata value may be included in one AtomicU8 operation, and extra shift and mask is required
    let res: usize = if metadata_spec.num_of_bits < BITS_IN_BYTE {
        debug_assert!(
            (metadata_spec.bit_offset >> LOG_BITS_IN_BYTE)
                == ((metadata_spec.bit_offset + metadata_spec.num_of_bits as isize - 1)
                    >> LOG_BITS_IN_BYTE),
            "Metadata << 8-bits: ({:?}) stretches over two bytes!",
            metadata_spec
        );
        let byte_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let bit_shift = metadata_spec.bit_offset - (byte_offset << LOG_BITS_IN_BYTE);
        let mask = ((1u8 << metadata_spec.num_of_bits) - 1) << bit_shift;

        let byte_val = unsafe {
            if let Some(order) = atomic_ordering {
                (object.to_address() + byte_offset).atomic_load::<AtomicU8>(order)
            } else {
                (object.to_address() + byte_offset).load::<u8>()
            }
        };

        ((byte_val & mask) >> bit_shift) as usize
    } else if metadata_spec.num_of_bits == BITS_IN_BYTE {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_BYTE.into(),
            "Metadata 16-bits: ({:?}) bit_offset must be byte aligned!",
            metadata_spec
        );
        let byte_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            if let Some(order) = atomic_ordering {
                (object.to_address() + byte_offset)
                    .atomic_load::<AtomicU8>(order)
                    .into()
            } else {
                (object.to_address() + byte_offset).load::<u8>().into()
            }
        }
    } else if metadata_spec.num_of_bits == BIT_IN_U16 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_U16,
            "Metadata 16-bits: ({:?}) bit_offset must be 2-bytes aligned!",
            metadata_spec
        );
        let u16_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            if let Some(order) = atomic_ordering {
                (object.to_address() + u16_offset)
                    .atomic_load::<AtomicU16>(order)
                    .into()
            } else {
                (object.to_address() + u16_offset).load::<u16>().into()
            }
        }
    } else if metadata_spec.num_of_bits == BITS_IN_U32 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_U32,
            "Metadata 32-bits: ({:?}) bit_offset must be 4-bytes aligned!",
            metadata_spec
        );
        let u32_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            if let Some(order) = atomic_ordering {
                (object.to_address() + u32_offset).atomic_load::<AtomicU32>(order) as usize
            } else {
                (object.to_address() + u32_offset).load::<u32>() as usize
            }
        }
    } else if metadata_spec.num_of_bits == BITS_IN_WORD {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_WORD,
            "Metadata 64-bits: ({:?}) bit_offset must be 8-bytes aligned!",
            metadata_spec
        );
        let u64_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            if let Some(order) = atomic_ordering {
                (object.to_address() + u64_offset).atomic_load::<AtomicUsize>(order)
            } else {
                (object.to_address() + u64_offset).load::<usize>()
            }
        }
    } else {
        unreachable!()
    };

    if let Some(mask) = optional_mask {
        res & mask
    } else {
        res
    }
}

/// This function provides a default implementation for the `store_metadata` method from the `ObjectModel` trait.
///
/// Note: this function does compare-and-swap in a busy loop. So, unlike `compare_exchange_metadata`, this operation will always success.
#[inline(always)]
pub fn store_metadata(
    metadata_spec: &HeaderMetadataSpec,
    object: ObjectReference,
    val: usize,
    optional_mask: Option<usize>,
    atomic_ordering: Option<Ordering>,
) {
    debug_assert!(optional_mask.is_none() || metadata_spec.num_of_bits >= 8,"optional_mask is only supported for 8X-bits in-header metadata. Problematic MetadataSpec: ({:?})", metadata_spec);

    // metadata smaller than 8-bits is special in that more than one metadata value may be included in one AtomicU8 operation, and extra shift and mask, and compare_exchange is required
    if metadata_spec.num_of_bits < 8 {
        debug_assert!(
            (metadata_spec.bit_offset >> LOG_BITS_IN_BYTE)
                == ((metadata_spec.bit_offset + metadata_spec.num_of_bits as isize - 1)
                    >> LOG_BITS_IN_BYTE),
            "Metadata << 8-bits: ({:?}) stretches over two bytes!",
            metadata_spec
        );
        let byte_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let bit_shift = metadata_spec.bit_offset - (byte_offset << LOG_BITS_IN_BYTE);
        let mask = ((1u8 << metadata_spec.num_of_bits) - 1) << bit_shift;

        let new_metadata = (val as u8) << bit_shift;
        let byte_addr = object.to_address() + byte_offset;
        if let Some(order) = atomic_ordering {
            unsafe {
                loop {
                    let old_byte_val = byte_addr.atomic_load::<AtomicU8>(order);
                    let new_byte_val = (old_byte_val & !mask) | new_metadata;
                    if byte_addr
                        .compare_exchange::<AtomicU8>(old_byte_val, new_byte_val, order, order)
                        .is_ok()
                    {
                        break;
                    }
                }
            }
        } else {
            unsafe {
                let old_byte_val = byte_addr.load::<u8>();
                let new_byte_val = (old_byte_val & !mask) | new_metadata;
                byte_addr.store::<u8>(new_byte_val);
            }
        }
    } else if metadata_spec.num_of_bits == 8 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_BYTE.into(),
            "Metadata 8-bits: ({:?}) bit_offset must be byte-aligned!",
            metadata_spec
        );
        let byte_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let byte_addr = object.to_address() + byte_offset;

        unsafe {
            if let Some(order) = atomic_ordering {
                // if the optional mask is provided (e.g. for forwarding pointer), we need to use compare_exchange
                if let Some(mask) = optional_mask {
                    loop {
                        let old_byte_val = byte_addr.atomic_load::<AtomicU8>(order);
                        let new_byte_val =
                            (old_byte_val & !(mask as u8)) | (val as u8 & (mask as u8));
                        if byte_addr
                            .compare_exchange::<AtomicU8>(old_byte_val, new_byte_val, order, order)
                            .is_ok()
                        {
                            break;
                        }
                    }
                } else {
                    byte_addr.atomic_store::<AtomicU8>(val as u8, order);
                }
            } else {
                let val = if let Some(mask) = optional_mask {
                    let old_byte_val = byte_addr.load::<u8>();
                    (old_byte_val & !(mask as u8)) | (val as u8 & (mask as u8))
                } else {
                    val as u8
                };
                byte_addr.store(val as u8);
            }
        }
    } else if metadata_spec.num_of_bits == 16 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_U16,
            "Metadata 16-bits: ({:?}) bit_offset must be 2-bytes aligned!",
            metadata_spec
        );
        let u16_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let u16_addr = object.to_address() + u16_offset;

        unsafe {
            if let Some(order) = atomic_ordering {
                // if the optional mask is provided (e.g. for forwarding pointer), we need to use compare_exchange
                if let Some(mask) = optional_mask {
                    loop {
                        let old_u16_val = u16_addr.atomic_load::<AtomicU16>(order);
                        let new_u16_val =
                            (old_u16_val & !(mask as u16)) | (val as u16 & (mask as u16));
                        if u16_addr
                            .compare_exchange::<AtomicU16>(old_u16_val, new_u16_val, order, order)
                            .is_ok()
                        {
                            break;
                        }
                    }
                } else {
                    u16_addr.atomic_store::<AtomicU16>(val as u16, order);
                }
            } else {
                let val = if let Some(mask) = optional_mask {
                    let old_byte_val = u16_addr.load::<u16>();
                    (old_byte_val & !(mask as u16)) | (val as u16 & (mask as u16))
                } else {
                    val as u16
                };

                u16_addr.store(val as u16);
            }
        }
    } else if metadata_spec.num_of_bits == 32 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_U32,
            "Metadata 32-bits: ({:?}) bit_offset must be 4-bytes aligned!",
            metadata_spec
        );
        let u32_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let u32_addr = object.to_address() + u32_offset;

        unsafe {
            if let Some(order) = atomic_ordering {
                // if the optional mask is provided (e.g. for forwarding pointer), we need to use compare_exchange
                if let Some(mask) = optional_mask {
                    loop {
                        let old_u32_val = u32_addr.atomic_load::<AtomicU32>(order);
                        let new_u32_val =
                            (old_u32_val & !(mask as u32)) | (val as u32 & (mask as u32));
                        if u32_addr
                            .compare_exchange::<AtomicU32>(old_u32_val, new_u32_val, order, order)
                            .is_ok()
                        {
                            break;
                        }
                    }
                } else {
                    u32_addr.atomic_store::<AtomicU32>(val as u32, order);
                }
            } else {
                let val = if let Some(mask) = optional_mask {
                    let old_byte_val = u32_addr.load::<u32>();
                    (old_byte_val & !(mask as u32)) | (val as u32 & (mask as u32))
                } else {
                    val as u32
                };

                u32_addr.store(val as u32);
            }
        }
    } else if metadata_spec.num_of_bits == 64 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_WORD,
            "Metadata 64-bits: ({:?}) bit_offset must be 8-bytes aligned!",
            metadata_spec
        );
        let u64_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let u64_addr = object.to_address() + u64_offset;

        unsafe {
            if let Some(order) = atomic_ordering {
                // if the optional mask is provided (e.g. for forwarding pointer), we need to use compare_exchange
                if let Some(mask) = optional_mask {
                    loop {
                        let old_val = u64_addr.atomic_load::<AtomicUsize>(order);
                        let new_val =
                            (old_val & !(mask as usize)) | (val as usize & (mask as usize));
                        if u64_addr
                            .compare_exchange::<AtomicUsize>(old_val, new_val, order, order)
                            .is_ok()
                        {
                            break;
                        }
                    }
                } else {
                    u64_addr.atomic_store::<AtomicUsize>(val as usize, order);
                }
            } else {
                let val = if let Some(mask) = optional_mask {
                    let old_val = u64_addr.load::<usize>();
                    (old_val & !(mask as usize)) | (val as usize & (mask as usize))
                } else {
                    val
                };

                u64_addr.store(val);
            }
        }
    } else {
        unreachable!()
    }
}

/// This function provides a default implementation for the `compare_exchange_metadata` method from the `ObjectModel` trait.
///
/// Note: this function only does fetch and exclusive store once, without any busy waiting in a loop.
#[inline(always)]
pub fn compare_exchange_metadata(
    metadata_spec: &HeaderMetadataSpec,
    object: ObjectReference,
    old_metadata: usize,
    new_metadata: usize,
    optional_mask: Option<usize>,
    success_order: Ordering,
    failure_order: Ordering,
) -> bool {
    // metadata smaller than 8-bits is special in that more than one metadata value may be included in one AtomicU8 operation, and extra shift and mask is required
    if metadata_spec.num_of_bits < 8 {
        debug_assert!(
            (metadata_spec.bit_offset >> LOG_BITS_IN_BYTE as isize)
                == ((metadata_spec.bit_offset + metadata_spec.num_of_bits as isize - 1)
                    >> LOG_BITS_IN_BYTE),
            "Metadata << 8-bits: ({:?}) stretches over two bytes!",
            metadata_spec
        );
        let byte_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let bit_shift = metadata_spec.bit_offset - (byte_offset << LOG_BITS_IN_BYTE);
        let mask = ((1u8 << metadata_spec.num_of_bits) - 1) << bit_shift;

        // let new_metadata = ((val as u8) << bit_shift);
        let byte_addr = object.to_address() + byte_offset;
        unsafe {
            let real_old_byte = byte_addr.atomic_load::<AtomicU8>(success_order);
            let expected_old_byte = (real_old_byte & !mask) | ((old_metadata as u8) << bit_shift);
            let expected_new_byte =
                (expected_old_byte & !mask) | ((new_metadata as u8) << bit_shift);
            byte_addr
                .compare_exchange::<AtomicU8>(
                    expected_old_byte,
                    expected_new_byte,
                    success_order,
                    failure_order,
                )
                .is_ok()
        }
    } else if metadata_spec.num_of_bits == 8 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_BYTE.into(),
            "Metadata 8-bits: ({:?}) bit_offset must be byte-aligned!",
            metadata_spec
        );
        let byte_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let byte_addr = object.to_address() + byte_offset;

        let (old_metadata, new_metadata) = if let Some(mask) = optional_mask {
            let old_byte = unsafe { byte_addr.atomic_load::<AtomicU8>(success_order) };
            let expected_new_byte = (old_byte & !(mask as u8)) | new_metadata as u8;
            let expected_old_byte = (old_byte & !(mask as u8)) | old_metadata as u8;
            (expected_old_byte, expected_new_byte)
        } else {
            (old_metadata as u8, new_metadata as u8)
        };

        unsafe {
            byte_addr
                .compare_exchange::<AtomicU8>(
                    old_metadata,
                    new_metadata,
                    success_order,
                    failure_order,
                )
                .is_ok()
        }
    } else if metadata_spec.num_of_bits == 16 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_U16,
            "Metadata 16-bits: ({:?}) bit_offset must be 2-bytes aligned!",
            metadata_spec
        );
        let u16_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let u16_addr = object.to_address() + u16_offset;

        let (old_metadata, new_metadata) = if let Some(mask) = optional_mask {
            let old_byte = unsafe { u16_addr.atomic_load::<AtomicU16>(success_order) };
            let expected_new_byte = (old_byte & !(mask as u16)) | new_metadata as u16;
            let expected_old_byte = (old_byte & !(mask as u16)) | old_metadata as u16;
            (expected_old_byte, expected_new_byte)
        } else {
            (old_metadata as u16, new_metadata as u16)
        };

        unsafe {
            u16_addr
                .compare_exchange::<AtomicU16>(
                    old_metadata,
                    new_metadata,
                    success_order,
                    failure_order,
                )
                .is_ok()
        }
    } else if metadata_spec.num_of_bits == 32 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_U32,
            "Metadata 32-bits: ({:?}) bit_offset must be 4-bytes aligned!",
            metadata_spec
        );
        let u32_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let u32_addr = object.to_address() + u32_offset;

        let (old_metadata, new_metadata) = if let Some(mask) = optional_mask {
            let old_byte = unsafe { u32_addr.atomic_load::<AtomicU32>(success_order) };
            let expected_new_byte = (old_byte & !(mask as u32)) | new_metadata as u32;
            let expected_old_byte = (old_byte & !(mask as u32)) | old_metadata as u32;
            (expected_old_byte, expected_new_byte)
        } else {
            (old_metadata as u32, new_metadata as u32)
        };

        unsafe {
            u32_addr
                .compare_exchange::<AtomicU32>(
                    old_metadata,
                    new_metadata,
                    success_order,
                    failure_order,
                )
                .is_ok()
        }
    } else if metadata_spec.num_of_bits == 64 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_WORD,
            "Metadata 64-bits: ({:?}) bit_offset must be 8-bytes aligned!",
            metadata_spec
        );
        let meta_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let meta_addr = object.to_address() + meta_offset;

        let (old_metadata, new_metadata) = if let Some(mask) = optional_mask {
            let old_val = unsafe { meta_addr.atomic_load::<AtomicUsize>(success_order) };
            let expected_new_val = (old_val & !mask) | new_metadata;
            let expected_old_val = (old_val & !mask) | old_metadata;
            (expected_old_val, expected_new_val)
        } else {
            (old_metadata, new_metadata)
        };

        unsafe {
            meta_addr
                .compare_exchange::<AtomicUsize>(
                    old_metadata,
                    new_metadata,
                    success_order,
                    failure_order,
                )
                .is_ok()
        }
    } else {
        unreachable!()
    }
}

/// This function provides a default implementation for the `fetch_add_metadata` method from the `ObjectModel` trait.
#[inline(always)]
pub fn fetch_add_metadata(
    metadata_spec: &HeaderMetadataSpec,
    object: ObjectReference,
    val: usize,
    order: Ordering,
) -> usize {
    // metadata smaller than 8-bits is special in that more than one metadata value may be included in one AtomicU8 operation, and extra shift and mask is required
    if metadata_spec.num_of_bits < 8 {
        debug_assert!(
            (metadata_spec.bit_offset >> LOG_BITS_IN_BYTE)
                == ((metadata_spec.bit_offset + metadata_spec.num_of_bits as isize - 1)
                    >> LOG_BITS_IN_BYTE),
            "Metadata << 8-bits: ({:?}) stretches over two bytes!",
            metadata_spec
        );
        let byte_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let bit_shift = metadata_spec.bit_offset - (byte_offset << LOG_BITS_IN_BYTE);
        let mask = ((1u8 << metadata_spec.num_of_bits) - 1) << bit_shift;

        // let new_metadata = ((val as u8) << bit_shift);
        let byte_addr = object.to_address() + byte_offset;
        loop {
            unsafe {
                let old_byte = byte_addr.atomic_load::<AtomicU8>(order);
                let old_metadata = (old_byte & mask) >> bit_shift;
                // new_metadata may contain overflow and should be and with the mask
                let new_metadata = (old_metadata + val as u8) & (mask >> bit_shift);
                let new_byte = (old_byte & !mask) | ((new_metadata as u8) << bit_shift);
                if byte_addr
                    .compare_exchange::<AtomicU8>(old_byte, new_byte, order, order)
                    .is_ok()
                {
                    return old_metadata as usize;
                }
            }
        }
    } else if metadata_spec.num_of_bits == 8 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_BYTE.into(),
            "Metadata 8-bits: ({:?}) bit_offset must be byte-aligned!",
            metadata_spec
        );
        let byte_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            (*(object.to_address() + byte_offset).to_ptr::<AtomicU8>())
                .fetch_add(val as u8, order)
                .into()
        }
    } else if metadata_spec.num_of_bits == 16 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_U16,
            "Metadata 16-bits: ({:?}) bit_offset must be 2-bytes aligned!",
            metadata_spec
        );
        let u16_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            (*(object.to_address() + u16_offset).to_ptr::<AtomicU16>())
                .fetch_add(val as u16, order)
                .into()
        }
    } else if metadata_spec.num_of_bits == 32 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_U32,
            "Metadata 32-bits: ({:?}) bit_offset must be 4-bytes aligned!",
            metadata_spec
        );
        let u32_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            (*(object.to_address() + u32_offset).to_ptr::<AtomicU32>()).fetch_add(val as u32, order)
                as usize
        }
    } else if metadata_spec.num_of_bits == 64 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_WORD,
            "Metadata 32-bits: ({:?}) bit_offset must be 4-bytes aligned!",
            metadata_spec
        );
        let meta_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            (*(object.to_address() + meta_offset).to_ptr::<AtomicUsize>()).fetch_add(val, order)
        }
    } else {
        unreachable!()
    }
}

/// This function provides a default implementation for the `fetch_sub_metadata` method from the `ObjectModel` trait.
#[inline(always)]
pub fn fetch_sub_metadata(
    metadata_spec: &HeaderMetadataSpec,
    object: ObjectReference,
    val: usize,
    order: Ordering,
) -> usize {
    // metadata smaller than 8-bits is special in that more than one metadata value may be included in one AtomicU8 operation, and extra shift and mask is required
    if metadata_spec.num_of_bits < 8 {
        debug_assert!(
            (metadata_spec.bit_offset >> LOG_BITS_IN_BYTE)
                == ((metadata_spec.bit_offset + metadata_spec.num_of_bits as isize - 1)
                    >> LOG_BITS_IN_BYTE),
            "Metadata << 8-bits: ({:?}) stretches over two bytes!",
            metadata_spec
        );
        let byte_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;
        let bit_shift = metadata_spec.bit_offset - (byte_offset << LOG_BITS_IN_BYTE);
        let mask = ((1u8 << metadata_spec.num_of_bits) - 1) << bit_shift;

        // let new_metadata = ((val as u8) << bit_shift);
        let byte_addr = object.to_address() + byte_offset;
        loop {
            unsafe {
                let old_byte = byte_addr.atomic_load::<AtomicU8>(order);
                let old_metadata = (old_byte & mask) >> bit_shift;
                // new_metadata may contain overflow and should be and with the mask
                let new_metadata = (old_metadata - val as u8) & (mask >> bit_shift);
                let new_byte = (old_byte & !mask) | ((new_metadata as u8) << bit_shift);
                if byte_addr
                    .compare_exchange::<AtomicU8>(old_byte, new_byte, order, order)
                    .is_ok()
                {
                    return old_metadata as usize;
                }
            }
        }
    } else if metadata_spec.num_of_bits == 8 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_BYTE.into(),
            "Metadata 8-bits: ({:?}) bit_offset must be byte-aligned!",
            metadata_spec
        );
        let byte_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            (*(object.to_address() + byte_offset).to_ptr::<AtomicU8>())
                .fetch_sub(val as u8, order)
                .into()
        }
    } else if metadata_spec.num_of_bits == 16 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_U16,
            "Metadata 16-bits: ({:?}) bit_offset must be 2-bytes aligned!",
            metadata_spec
        );
        let u16_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            (*(object.to_address() + u16_offset).to_ptr::<AtomicU16>())
                .fetch_sub(val as u16, order)
                .into()
        }
    } else if metadata_spec.num_of_bits == 32 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_U32,
            "Metadata 32-bits: ({:?}) bit_offset must be 4-bytes aligned!",
            metadata_spec
        );
        let u32_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            (*(object.to_address() + u32_offset).to_ptr::<AtomicU32>()).fetch_sub(val as u32, order)
                as usize
        }
    } else if metadata_spec.num_of_bits == 64 {
        debug_assert!(
            metadata_spec.bit_offset.trailing_zeros() as usize >= LOG_BITS_IN_WORD,
            "Metadata 32-bits: ({:?}) bit_offset must be 4-bytes aligned!",
            metadata_spec
        );
        let meta_offset = metadata_spec.bit_offset >> LOG_BITS_IN_BYTE;

        unsafe {
            (*(object.to_address() + meta_offset).to_ptr::<AtomicUsize>()).fetch_sub(val, order)
        }
    } else {
        unreachable!()
    }
}
