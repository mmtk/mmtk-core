/*
 * Provides a way to adjust size limits for the boot image.
 * <p>
 * It would probably a good idea to have a systematic way to
 * set the expected size of the boot image segments, depending
 * on the architecture. We currently don't have that.
 * <p>
 * The approach for the initial version of this class
 * was to adjust the limits that have been in place for a while
 * for the architectures supported at that time:
 * <ul>
 *   <li>64 bit boot images are expected to have a larger data
 *       segment than 32 bit images due to 64 bit pointers.</li>
 *   <li>x64 code is significantly larger than x86 code</li>
 * </ul>
 */

// Data from Nov 2016 shows that data size grows by
// about a third for development builds for
// x86 -> x64 and PPC32 -> PPC64

#[cfg(target_pointer_width = "32")]
pub const DATA_SIZE_ADJUSTMENT: f32 = 1.0;

#[cfg(target_pointer_width = "64")]
pub const DATA_SIZE_ADJUSTMENT: f32 = 1.35;

// x64 code is a lot bigger than ia32 code.
// For PPC, code size growth from 32 bit to 64 bit is
// not nearly as big. The current limits are fine for PPC
// so no adjustment is needed.

#[cfg(target_arch = "x86_64")]
pub const CODE_SIZE_ADJUSTMENT: f32 = 1.5;

#[cfg(not(target_arch = "x86_64"))]
pub const CODE_SIZE_ADJUSTMENT: f32 = 1.0;

