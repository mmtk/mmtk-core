use atomic::Atomic;
use mmtk::{
    util::{Address, ObjectReference},
    vm::edge_shape::{Edge, SimpleEdge},
};

/// If a VM supports multiple kinds of edges, we can use tagged union to represent all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DummyVMEdge {
    Simple(SimpleEdge),
    #[cfg(target_pointer_width = "64")]
    Compressed(only_64_bit::CompressedOopEdge),
    Offset(OffsetEdge),
    Tagged(TaggedEdge),
}

unsafe impl Send for DummyVMEdge {}

impl Edge for DummyVMEdge {
    fn from_address(_address: Address) -> Self {
        unimplemented!()
    }

    fn load(&self) -> ObjectReference {
        match self {
            DummyVMEdge::Simple(e) => e.load(),
            #[cfg(target_pointer_width = "64")]
            DummyVMEdge::Compressed(e) => e.load(),
            DummyVMEdge::Offset(e) => e.load(),
            DummyVMEdge::Tagged(e) => e.load(),
        }
    }

    fn store(&self, object: ObjectReference) {
        match self {
            DummyVMEdge::Simple(e) => e.store(object),
            #[cfg(target_pointer_width = "64")]
            DummyVMEdge::Compressed(e) => e.store(object),
            DummyVMEdge::Offset(e) => e.store(object),
            DummyVMEdge::Tagged(e) => e.store(object),
        }
    }
}

/// Compressed OOP edge only makes sense on 64-bit architectures.
#[cfg(target_pointer_width = "64")]
pub mod only_64_bit {
    use super::*;

    /// This represents a location that holds a 32-bit pointer on a 64-bit machine.
    ///
    /// OpenJDK uses this kind of edge to store compressed OOPs on 64-bit machines.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct CompressedOopEdge {
        slot_addr: *mut Atomic<u32>,
    }

    unsafe impl Send for CompressedOopEdge {}

    impl CompressedOopEdge {
        pub fn from_address(address: Address) -> Self {
            Self {
                slot_addr: address.to_mut_ptr(),
            }
        }
        pub fn as_address(&self) -> Address {
            Address::from_mut_ptr(self.slot_addr)
        }
    }

    impl Edge for CompressedOopEdge {
        fn from_address(_address: Address) -> Self {
            unimplemented!()
        }

        fn load(&self) -> ObjectReference {
            let compressed = unsafe { (*self.slot_addr).load(atomic::Ordering::Relaxed) };
            let expanded = (compressed as usize) << 3;
            unsafe { Address::from_usize(expanded).to_object_reference() }
        }

        fn store(&self, object: ObjectReference) {
            let expanded = object.to_address().as_usize();
            let compressed = (expanded >> 3) as u32;
            unsafe { (*self.slot_addr).store(compressed, atomic::Ordering::Relaxed) }
        }
    }
}

/// This represents an edge that holds a pointer to the *middle* of an object, and the offset is known.
///
/// Julia uses this trick to facilitate deleting array elements from the front.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OffsetEdge {
    slot_addr: *mut Atomic<Address>,
    offset: usize,
}

unsafe impl Send for OffsetEdge {}

impl OffsetEdge {
    pub fn new_no_offset(address: Address) -> Self {
        Self {
            slot_addr: address.to_mut_ptr(),
            offset: 0,
        }
    }

    pub fn new_with_offset(address: Address, offset: usize) -> Self {
        Self {
            slot_addr: address.to_mut_ptr(),
            offset,
        }
    }

    pub fn slot_address(&self) -> Address {
        Address::from_mut_ptr(self.slot_addr)
    }

    pub fn offset(&self) -> usize {
        self.offset
    }
}

impl Edge for OffsetEdge {
    fn from_address(_address: Address) -> Self {
        unimplemented!()
    }

    fn load(&self) -> ObjectReference {
        let middle = unsafe { (*self.slot_addr).load(atomic::Ordering::Relaxed) };
        let begin = middle - self.offset;
        unsafe { begin.to_object_reference() }
    }

    fn store(&self, object: ObjectReference) {
        let begin = object.to_address();
        let middle = begin + self.offset;
        unsafe { (*self.slot_addr).store(middle, atomic::Ordering::Relaxed) }
    }
}

/// This edge presents the object reference itself to mmtk-core.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaggedEdge {
    slot_addr: *mut Atomic<usize>,
}

unsafe impl Send for TaggedEdge {}

impl TaggedEdge {
    // The DummyVM has OBJECT_REF_OFFSET = 4.
    // Using a two-bit tag should be safe on both 32-bit and 64-bit platforms.
    const TAG_BITS_MASK: usize = 0b11;

    #[inline(always)]
    pub fn new(address: Address) -> Self {
        Self {
            slot_addr: address.to_mut_ptr(),
        }
    }
}

impl Edge for TaggedEdge {
    fn from_address(_address: Address) -> Self {
        unimplemented!()
    }

    fn load(&self) -> ObjectReference {
        let tagged = unsafe { (*self.slot_addr).load(atomic::Ordering::Relaxed) };
        let untagged = tagged & !Self::TAG_BITS_MASK;
        unsafe { Address::from_usize(untagged).to_object_reference() }
    }

    fn store(&self, object: ObjectReference) {
        let old_tagged = unsafe { (*self.slot_addr).load(atomic::Ordering::Relaxed) };
        let new_untagged = object.to_address().as_usize();
        let new_tagged = new_untagged | (old_tagged & Self::TAG_BITS_MASK);
        unsafe { (*self.slot_addr).store(new_tagged, atomic::Ordering::Relaxed) }
    }
}
