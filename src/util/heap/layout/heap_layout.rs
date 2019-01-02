use util::heap::layout::ByteMapMmapper;
use util::heap::layout::map32::Map32;

// FIXME: Use FragmentMmapper for 64-bit heaps
lazy_static! {
    pub static ref MMAPPER: ByteMapMmapper = ByteMapMmapper::new();
    pub static ref VM_MAP: Map32 = Map32::new();
}