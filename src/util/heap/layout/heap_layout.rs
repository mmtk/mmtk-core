use util::heap::layout::ByteMapMmapper;

// FIXME: Use FragmentMmapper for 64-bit heaps
lazy_static! {
    pub static ref MMAPPER: ByteMapMmapper = ByteMapMmapper::new();
}