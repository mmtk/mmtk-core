use vm::jikesrvm::boot_image_size::DATA_SIZE_ADJUSTMENT;
use vm::jikesrvm::boot_image_size::CODE_SIZE_ADJUSTMENT;
use util::Address;

/** The traditional 32-bit heap layout */
pub const HEAP_LAYOUT_32BIT: usize = 1;

/** The 64-bit heap layout that allows heap sizes up to 2^42 bytes */
pub const HEAP_LAYOUT_64BIT: usize = 2;

/** Choose between the possible heap layout styles */
#[cfg(target_pointer_width = "32")]
pub const HEAP_LAYOUT: usize = HEAP_LAYOUT_32BIT;

/** Choose between the possible heap layout styles */
#[cfg(target_pointer_width = "64")]
pub const HEAP_LAYOUT: usize = HEAP_LAYOUT_32BIT;

/** The address of the start of the data section of the boot image. */
pub const BOOT_IMAGE_DATA_START: Address = unsafe { Address::from_usize(0x60000000) };

/** The address of the start of the code section of the boot image. */
pub const BOOT_IMAGE_CODE_START: Address = unsafe { Address::from_usize(0x64000000) };

/** The address of the start of the ref map section of the boot image. */
pub const BOOT_IMAGE_RMAP_START: Address = unsafe { Address::from_usize(0x67000000) };

/** The address in virtual memory that is the highest that can be mapped. */
pub const MAXIMUM_MAPPABLE: Address = unsafe { Address::from_usize(0xb0000000) };

/** The current boot image data size */
pub const BOOT_IMAGE_DATA_SIZE: usize = BOOT_IMAGE_CODE_START.as_usize() - BOOT_IMAGE_DATA_START.as_usize();

/** The current boot image code size */
pub const BOOT_IMAGE_CODE_SIZE: usize = BOOT_IMAGE_RMAP_START.as_usize() - BOOT_IMAGE_CODE_START.as_usize();

/**
 * Limit for boot image data size: fail the build if
 * {@link org.jikesrvm.Configuration#AllowOversizedImages VM.AllowOversizedImages}
 * is not set and the boot image data size is greater than or equal to this amount
 * of bytes.
 */
pub const BOOT_IMAGE_DATA_SIZE_LIMIT: usize = (1.0 * (56 << 20) as f32 * DATA_SIZE_ADJUSTMENT) as usize;

/**
 * Limit for boot image code size: fail the build if
 * {@link org.jikesrvm.Configuration#AllowOversizedImages VM.AllowOversizedImages}
 * is not set and the boot image code size is greater than or equal to this amount
 * of bytes.
 */
// TODO Changed the limit from 24 << 20 to 24 << 21, need to check if this has unintended side effects
pub const BOOT_IMAGE_CODE_SIZE_LIMIT: usize = (1.0 * (24 << 21) as f32 * CODE_SIZE_ADJUSTMENT) as usize;

/* Typical compression ratio is about 1/20 */
pub const BAD_MAP_COMPRESSION: usize = 5;  // conservative heuristic
pub const MAX_BOOT_IMAGE_RMAP_SIZE: usize = BOOT_IMAGE_DATA_SIZE/BAD_MAP_COMPRESSION;

/** The address of the end of the data section of the boot image. */
pub const BOOT_IMAGE_DATA_END: Address = unsafe {
    Address::from_usize(BOOT_IMAGE_DATA_START.as_usize() + BOOT_IMAGE_DATA_SIZE)
};
/** The address of the end of the code section of the boot image. */
pub const BOOT_IMAGE_CODE_END: Address = unsafe {
    Address::from_usize(BOOT_IMAGE_CODE_START.as_usize() + BOOT_IMAGE_CODE_SIZE)
};
/** The address of the end of the ref map section of the boot image. */
pub const BOOT_IMAGE_RMAP_END: Address = unsafe {
    Address::from_usize(BOOT_IMAGE_RMAP_START.as_usize() + MAX_BOOT_IMAGE_RMAP_SIZE)
};
/** The address of the end of the boot image. */
pub const BOOT_IMAGE_END: Address = BOOT_IMAGE_RMAP_END;