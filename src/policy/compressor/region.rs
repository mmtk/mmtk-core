use crate::util::linear_scan::Region;
use crate::util::Address;

#[derive(Copy, Clone, PartialEq, PartialOrd)]
pub(crate) struct CompressorRegion(Address);
impl Region for CompressorRegion {
    const LOG_BYTES: usize = 20; // 1 MiB
    fn from_aligned_address(address: Address) -> Self {
        assert!(address.is_aligned_to(Self::BYTES), "{address} is not aligned");
        CompressorRegion(address)
    }
    fn start(&self) -> Address {
        self.0
    }
}
