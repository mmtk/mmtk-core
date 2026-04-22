use super::mock_test_prelude::*;
use crate::util::alloc::*;
use std::mem::size_of;

#[test]
pub fn test_option_allocator_layout() {
    assert_eq!(size_of::<Option<BumpAllocator<MockVM>>>(), size_of::<BumpAllocator<MockVM>>());
    assert_eq!(size_of::<Option<LargeObjectAllocator<MockVM>>>(), size_of::<LargeObjectAllocator<MockVM>>());
    assert_eq!(size_of::<Option<MallocAllocator<MockVM>>>(), size_of::<MallocAllocator<MockVM>>());
    assert_eq!(size_of::<Option<ImmixAllocator<MockVM>>>(), size_of::<ImmixAllocator<MockVM>>());
    assert_eq!(size_of::<Option<FreeListAllocator<MockVM>>>(), size_of::<FreeListAllocator<MockVM>>());
    assert_eq!(size_of::<Option<MarkCompactAllocator<MockVM>>>(), size_of::<MarkCompactAllocator<MockVM>>());
}

#[test]
pub fn test_0_transmute_as_option_none() {
    const BUMP_ALLOCATOR_SIZE: usize = size_of::<Option<BumpAllocator<MockVM>>>();
    let zero_bump = unsafe { std::mem::transmute::<_, Option<BumpAllocator<MockVM>>>([0u8; BUMP_ALLOCATOR_SIZE]) };
    assert!(zero_bump.is_none());
}
