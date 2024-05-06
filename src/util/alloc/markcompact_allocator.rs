use std::sync::Arc;

use super::allocator::AllocatorContext;
use super::BumpAllocator;
use crate::policy::space::Space;
use crate::util::alloc::Allocator;
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::vm::VMBinding;

/// A thin wrapper(specific implementation) of bump allocator
/// reserve extra bytes when allocating
#[repr(C)]
pub struct MarkCompactAllocator<VM: VMBinding> {
    pub(in crate::util::alloc) bump_allocator: BumpAllocator<VM>,
}

impl<VM: VMBinding> MarkCompactAllocator<VM> {
    pub(crate) fn set_limit(&mut self, cursor: Address, limit: Address) {
        self.bump_allocator.set_limit(cursor, limit);
    }

    pub(crate) fn reset(&mut self) {
        self.bump_allocator.reset();
    }

    pub(crate) fn rebind(&mut self, space: &'static dyn Space<VM>) {
        self.bump_allocator.rebind(space);
    }
}

impl<VM: VMBinding> Allocator<VM> for MarkCompactAllocator<VM> {
    fn get_space(&self) -> &'static dyn Space<VM> {
        self.bump_allocator.get_space()
    }

    fn get_context(&self) -> &AllocatorContext<VM> {
        &self.bump_allocator.context
    }

    fn get_tls(&self) -> VMThread {
        self.bump_allocator.get_tls()
    }

    fn does_thread_local_allocation(&self) -> bool {
        true
    }

    fn get_thread_local_buffer_granularity(&self) -> usize {
        self.bump_allocator.get_thread_local_buffer_granularity()
    }

    fn alloc(&mut self, size: usize, align: usize, offset: usize) -> Address {
        let rtn = self
            .bump_allocator
            .alloc(size + Self::HEADER_RESERVED_IN_BYTES, align, offset);
        // Check if the result is valid and return the actual object start address
        // Note that `rtn` can be null in the case of OOM
        if !rtn.is_zero() {
            rtn + Self::HEADER_RESERVED_IN_BYTES
        } else {
            rtn
        }
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: usize) -> Address {
        trace!("alloc_slow");
        self.bump_allocator.alloc_slow_once(size, align, offset)
    }

    /// Slow path for allocation if precise stress testing has been enabled.
    /// It works by manipulating the limit to be always below the cursor.
    /// Can have three different cases:
    ///  - acquires a new block if the hard limit has been met;
    ///  - allocates an object using the bump pointer semantics from the
    ///    fastpath if there is sufficient space; and
    ///  - does not allocate an object but forces a poll for GC if the stress
    ///    factor has been crossed.
    fn alloc_slow_once_precise_stress(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        need_poll: bool,
    ) -> Address {
        self.bump_allocator
            .alloc_slow_once_precise_stress(size, align, offset, need_poll)
    }
}

impl<VM: VMBinding> MarkCompactAllocator<VM> {
    /// The number of bytes that the allocator reserves for its own header.
    pub const HEADER_RESERVED_IN_BYTES: usize =
        crate::policy::markcompactspace::MarkCompactSpace::<VM>::HEADER_RESERVED_IN_BYTES;
    pub(crate) fn new(
        tls: VMThread,
        space: &'static dyn Space<VM>,
        context: Arc<AllocatorContext<VM>>,
    ) -> Self {
        MarkCompactAllocator {
            bump_allocator: BumpAllocator::new(tls, space, context),
        }
    }
}
