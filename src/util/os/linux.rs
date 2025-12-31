use crate::util::os::memory::*;
use crate::util::address::Address;
use std::io::Result;

pub struct LinuxMemory;

impl Memory for LinuxMemory {
    fn dzmmap(start: Address, size: usize, strategy: MmapStrategy, annotation: &MmapAnnotation<'_>) -> Result<Address> {
        // Windows-specific implementation of dzmmap
        unimplemented!()
    }

    fn munmap(start: Address, size: usize) -> Result<()> {
        // Windows-specific implementation of munmap
        unimplemented!()
    }

    fn mprotect(start: Address, size: usize) -> Result<()> {
        // Windows-specific implementation of mprotect
        unimplemented!()
    }

    fn munprotect(start: Address, size: usize) -> Result<()> {
        // Windows-specific implementation of munprotect
        unimplemented!()
    }
}
