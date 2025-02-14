//! Track memory ranges for tools like Valgrind address sanitizer, or other memory checkers.

use crate::util::Address;

pub fn tracking_enabled() -> bool {
    #[cfg(feature = "crabgrind")]
    {
        crabgrind::run_mode() != crabgrind::RunMode::Native
    }

    #[cfg(not(feature = "crabgrind"))]
    {
        false
    }
}

pub fn track_malloc(p: Address, size: usize, zero: bool) {
    #[cfg(feature = "crabgrind")]
    {
        crabgrind::memcheck::alloc::malloc(p.to_mut_ptr(), size, 0, zero)
    }

    #[cfg(not(feature = "crabgrind"))]
    {
        let _ = p;
        let _ = size;
        let _ = zero;
    }
}

pub fn track_free(p: Address, size: usize) {
    let _ = size;
    #[cfg(feature = "crabgrind")]
    {
        crabgrind::memcheck::alloc::free(p.to_mut_ptr(), 0);
    }
    #[cfg(not(feature = "crabgrind"))]
    {
        let _ = p;
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MemState {
    NoAccess,
    Undefined,
    Defined,
    DefinedIfAddressable,
}

pub fn track_mem(p: Address, size: usize, state: MemState) {
    #[cfg(feature = "crabgrind")]
    {
        let state = match state {
            MemState::Defined => crabgrind::memcheck::MemState::Defined,
            MemState::DefinedIfAddressable => crabgrind::memcheck::MemState::DefinedIfAddressable,
            MemState::NoAccess => crabgrind::memcheck::MemState::NoAccess,
            MemState::Undefined => crabgrind::memcheck::MemState::Undefined,
        };

        crabgrind::memcheck::mark_mem(p.to_mut_ptr(), size, state);
    }

    #[cfg(not(feature = "crabgrind"))]
    {
        let _ = p;
        let _ = size;
        let _ = state;
    }
}

/// Track a memory pool. Read [Memory Pools](https://valgrind.org/docs/manual/mc-manual.html#mc-manual.mempools)
/// of valgrind for more information.
///
/// # Parameters
///
/// - `pool`: The memory pool to track.
/// - `redzone`: Redzone in between chunks.
/// - `is_zeroed`: Whether the memory pool is zeroed.
pub fn track_mempool<T>(pool: &T, redzone: usize, is_zeroed: bool) {
    #[cfg(feature = "crabgrind")]
    {
        crabgrind::memcheck::mempool::create(
            Address::from_ref(pool).to_mut_ptr(),
            redzone,
            is_zeroed,
            Some(crabgrind::memcheck::mempool::AUTO_FREE | crabgrind::memcheck::mempool::METAPOOL),
        );
    }

    #[cfg(not(feature = "crabgrind"))]
    {
        let _ = pool;
        let _ = redzone;
        let _ = is_zeroed;
    }
}

/// Untrack a memory pool. This destroys the memory pool in the memory checker.
pub fn untrack_mempool<T>(pool: &T) {
    #[cfg(feature = "crabgrind")]
    {
        crabgrind::memcheck::mempool::destroy(Address::from_ref(pool).to_mut_ptr());
    }
    #[cfg(not(feature = "crabgrind"))]
    {
        let _ = pool;
    }
}

/// Associate a piece of memory with a memory pool.
///
/// # Parameters    
/// - `pool`: The memory pool to associate with.
/// - `addr`: The address of the memory to associate.
/// - `size`: The size of the memory to associate.
pub fn track_mempool_alloc<T>(pool: &T, addr: Address, size: usize) {

    #[cfg(feature = "crabgrind")]
    {
        crabgrind::memcheck::mempool::alloc(
            Address::from_ptr(pool as *const T as *const u8).to_mut_ptr(),
            addr.to_mut_ptr(),
            size,
        );
    }

    #[cfg(not(feature = "crabgrind"))]
    {
        let _ = pool;
        let _ = addr;
        let _ = size;
    }
}

/// Disassociate a piece of memory with a memory pool.
///
/// # Parameters
/// - `pool`: The memory pool to disassociate with.
/// - `addr`: The address of the memory to disassociate.
pub fn track_mempool_free<T>(pool: &T, addr: Address) {
    #[cfg(feature = "crabgrind")]
    {
        crabgrind::memcheck::mempool::free(Address::from_ref(pool).to_mut_ptr(), addr.to_mut_ptr());
    }
    #[cfg(not(feature = "crabgrind"))]
    {
        let _ = pool;
        let _ = addr;
    }
}
