use crate::util::linear_scan::Region;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::metadata::side_metadata::spec_defs::COMPRESSOR_REGION_USAGE;
use crate::util::Address;
use atomic::Ordering;

#[derive(Copy, Clone, PartialEq, PartialOrd)]
pub(crate) struct CompressorRegion(Address);
impl Region for CompressorRegion {
    const LOG_BYTES: usize = 19; // 512 kiB
    fn from_aligned_address(address: Address) -> Self {
        assert!(address.is_aligned_to(Self::BYTES));
        CompressorRegion(address)
    }
    fn start(&self) -> Address {
        self.0
    }
}

impl CompressorRegion {
    pub const REGION_USAGE_SPEC: SideMetadataSpec = COMPRESSOR_REGION_USAGE;
    // Same as crate::util::alloc::bumpallocator::BLOCK_SIZE
    pub const TLAB_BYTES: usize = 8 << crate::util::constants::LOG_BYTES_IN_PAGE;
    pub fn usage(&self) -> Address {
        let addr = Self::REGION_USAGE_SPEC.load_atomic::<usize>(self.0, Ordering::Relaxed);
        unsafe { Address::from_usize(addr) }
    }
    pub fn set_usage(&self, usage: Address) {
        debug_assert_eq!(self.0, CompressorRegion::from_unaligned_address(usage).0);
        Self::REGION_USAGE_SPEC.store_atomic::<usize>(self.0, usage.as_usize(), Ordering::Relaxed);
    }
    pub fn compare_exchange_usage(&self, old: Address, new: Address) -> bool {
        Self::REGION_USAGE_SPEC.compare_exchange_atomic::<usize>(
            self.0,
            old.as_usize(),
            new.as_usize(),
            Ordering::SeqCst,
            Ordering::SeqCst,
        ).is_ok()
    }
    pub fn initialise(&self) {
        self.set_usage(self.0);
    }
    pub fn allocate_tlab(&self) -> Option<Address> {
        loop {
            let old_usage = self.usage();
            let free = old_usage.align_up(Self::TLAB_BYTES);
            if free >= self.end() {
                return Option::None;
            } else {
                let new_usage = free + Self::TLAB_BYTES;
                if self.compare_exchange_usage(old_usage, new_usage) {
                    return Option::Some(free);
                }
            }
        }
    }
}
