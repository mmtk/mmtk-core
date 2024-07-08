use criterion::Criterion;

use mmtk::memory_manager;
use mmtk::util::test_util::fixtures::*;
use mmtk::util::test_util::mock_method::*;
use mmtk::util::test_util::mock_vm::{write_mockvm, MockVM};
use mmtk::AllocationSemantics;

pub fn bench(c: &mut Criterion) {
    // Setting a larger heap, although the GC should be disabled in the MockVM
    let mut fixture = MutatorFixture::create_with_heapsize(1 << 30);

    // Normal objects
    // 16KB object -- we want to make sure the object can fit into any normal space (e.g. immix space or mark sweep space)
    const NORMAL_OBJECT_SIZE: usize = 16 * 1024;
    write_mockvm(|mock| {
        *mock = MockVM {
            get_object_size: MockMethod::new_fixed(Box::new(|_| NORMAL_OBJECT_SIZE)),
            is_collection_enabled: MockMethod::new_fixed(Box::new(|_| false)),
            ..MockVM::default()
        }
    });

    c.bench_function("internal pointer - normal objects", |b| {
        #[cfg(feature = "is_mmtk_object")]
        {
            use mmtk::vm::ObjectModel;
            let addr = memory_manager::alloc(&mut fixture.mutator, NORMAL_OBJECT_SIZE, 8, 0, AllocationSemantics::Default);
            let obj_ref = MockVM::address_to_ref(addr);
            memory_manager::post_alloc(&mut fixture.mutator, obj_ref, NORMAL_OBJECT_SIZE, AllocationSemantics::Default);
            let obj_end = addr + NORMAL_OBJECT_SIZE;
            b.iter(|| {
                memory_manager::find_object_from_internal_pointer::<MockVM>(obj_end - 1, NORMAL_OBJECT_SIZE);
            })
        }
        #[cfg(not(feature = "is_mmtk_object"))]
        panic!("The benchmark requires is_mmtk_object feature to run");
    });

    // Large objects
    // 16KB object
    const LARGE_OBJECT_SIZE: usize = 16 * 1024;
    write_mockvm(|mock| {
        *mock = MockVM {
            get_object_size: MockMethod::new_fixed(Box::new(|_| LARGE_OBJECT_SIZE)),
            is_collection_enabled: MockMethod::new_fixed(Box::new(|_| false)),
            ..MockVM::default()
        }
    });
    c.bench_function("internal pointer - large objects", |b| {
        #[cfg(feature = "is_mmtk_object")]
        {
            use mmtk::vm::ObjectModel;
            let addr = memory_manager::alloc(&mut fixture.mutator, LARGE_OBJECT_SIZE, 8, 0, AllocationSemantics::Los);
            let obj_ref = MockVM::address_to_ref(addr);
            memory_manager::post_alloc(&mut fixture.mutator, obj_ref, LARGE_OBJECT_SIZE, AllocationSemantics::Los);
            let obj_end = addr + LARGE_OBJECT_SIZE;
            b.iter(|| {
                memory_manager::find_object_from_internal_pointer::<MockVM>(obj_end - 1, LARGE_OBJECT_SIZE);
            })
        }
        #[cfg(not(feature = "is_mmtk_object"))]
        panic!("The benchmark requires is_mmtk_object feature to run");
    });
}
