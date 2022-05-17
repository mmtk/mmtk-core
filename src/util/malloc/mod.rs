pub(crate) mod library;
pub mod malloc_ms_util;

use crate::util::Address;
use crate::vm::VMBinding;
use crate::MMTK;

#[inline(always)]
pub fn malloc(size: usize) -> Address {
    Address::from_mut_ptr(unsafe { self::library::malloc(size) })
}

#[inline(always)]
pub fn calloc(num: usize, size: usize) -> Address {
    Address::from_mut_ptr(unsafe { self::library::calloc(num, size) })
}

#[inline(always)]
pub fn realloc(addr: Address, size: usize) -> Address {
    Address::from_mut_ptr(unsafe { self::library::realloc(addr.to_mut_ptr(), size) })
}

#[inline(always)]
pub fn free(addr: Address) {
    unsafe { self::library::free(addr.to_mut_ptr()) }
}

impl<VM: VMBinding> MMTK<VM> {
    #[inline(always)]
    pub fn malloc(&self, size: usize) -> Address {
        #[cfg(feature = "malloc_counted_size")]
        self.plan.base().increase_malloc_bytes_by(size);

        Address::from_mut_ptr(unsafe { self::library::malloc(size) })
    }

    #[inline(always)]
    pub fn calloc(&self, num: usize, size: usize) -> Address {
        #[cfg(feature = "malloc_counted_size")]
        self.plan.base().increase_malloc_bytes_by(num * size);

        Address::from_mut_ptr(unsafe { self::library::calloc(num, size) })
    }
}

#[cfg(feature = "malloc_counted_size")]
impl<VM: VMBinding> MMTK<VM> {
    #[inline(always)]
    pub fn realloc_with_old_size(&self, addr: Address, size: usize, old_size: usize) -> Address {
        let base_plan = self.plan.base();
        base_plan.decrease_malloc_bytes_by(old_size);
        base_plan.increase_malloc_bytes_by(size);

        Address::from_mut_ptr(unsafe { self::library::realloc(addr.to_mut_ptr(), size) })
    }

    #[inline(always)]
    pub fn free_with_size(&self, addr: Address, size: usize) {
        self.plan.base().decrease_malloc_bytes_by(size);
        unsafe { self::library::free(addr.to_mut_ptr()) }
    }
}

#[cfg(not(feature = "malloc_counted_size"))]
impl<VM: VMBinding> MMTK<VM> {
    #[inline(always)]
    pub fn realloc(&self, addr: Address, size: usize) -> Address {
        Address::from_mut_ptr(unsafe { self::library::realloc(addr.to_mut_ptr(), size) })
    }

    #[inline(always)]
    pub fn free(&self, addr: Address) {
        unsafe { self::library::free(addr.to_mut_ptr()) }
    }
}
