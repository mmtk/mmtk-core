// GITHUB-CI: MMTK_PLAN=NoGC,SemiSpace,Immix,GenImmix,StickyImmix

use crate::test_fixtures::{MMTKSingleton, SerialFixture};
use crate::DummyVM;

use mmtk::util::Address;
use mmtk::util::OpaquePointer;
use mmtk::util::{VMMutatorThread, VMThread};
use mmtk::AllocationSemantics;
use mmtk::Mutator;

lazy_static! {
    static ref MMTK_SINGLETON: SerialFixture<MMTKSingleton> = SerialFixture::new();
}

#[test]
pub fn boxed_pointer() {
    MMTK_SINGLETON.with_fixture(|fixture| {
        let tls_opaque_pointer = VMMutatorThread(VMThread(OpaquePointer::UNINITIALIZED));

        // ANCHOR: mutator_storage_boxed_pointer
        struct MutatorInTLS {
            // Store the mutator as a boxed pointer.
            // Accessing any value in the mutator will need a dereferencing of the boxed pointer.
            ptr: Box<Mutator<DummyVM>>,
        }

        // Bind an MMTk mutator
        let mutator = mmtk::memory_manager::bind_mutator(&fixture.mmtk, tls_opaque_pointer);
        // Store the pointer in TLS
        let mut storage = MutatorInTLS { ptr: mutator };

        // Allocate
        let addr =
            mmtk::memory_manager::alloc(&mut storage.ptr, 8, 8, 0, AllocationSemantics::Default);
        // ANCHOR_END: mutator_storage_boxed_pointer

        assert!(!addr.is_zero());
    });
}

#[test]
pub fn embed_mutator_struct() {
    MMTK_SINGLETON.with_fixture(|fixture| {
        let tls_opaque_pointer = VMMutatorThread(VMThread(OpaquePointer::UNINITIALIZED));

        // ANCHOR: mutator_storage_embed_mutator_struct
        struct MutatorInTLS {
            embed: Mutator<DummyVM>,
        }

        // Bind an MMTk mutator
        let mutator = mmtk::memory_manager::bind_mutator(&fixture.mmtk, tls_opaque_pointer);
        // Store the struct (or use memcpy for non-Rust code)
        let mut storage = MutatorInTLS { embed: *mutator };
        // Allocate
        let addr =
            mmtk::memory_manager::alloc(&mut storage.embed, 8, 8, 0, AllocationSemantics::Default);
        // ANCHOR_END: mutator_storage_embed_mutator_struct

        assert!(!addr.is_zero());
    })
}

#[test]
pub fn embed_fastpath_struct() {
    MMTK_SINGLETON.with_fixture(|fixture| {
        let tls_opaque_pointer = VMMutatorThread(VMThread(OpaquePointer::UNINITIALIZED));

        // ANCHOR: mutator_storage_embed_fastpath_struct
        use mmtk::util::alloc::BumpPointer;
        struct MutatorInTLS {
            default_bump_pointer: BumpPointer,
            mutator: Box<Mutator<DummyVM>>,
        }

        // Bind an MMTk mutator
        let mutator = mmtk::memory_manager::bind_mutator(&fixture.mmtk, tls_opaque_pointer);
        // Create a fastpath BumpPointer with default(). The BumpPointer from default() will guarantee to fail on the first allocation
        // so the allocation goes to the slowpath and we will get an allocation buffer from MMTk.
        let default_bump_pointer = BumpPointer::default();
        // Store the fastpath BumpPointer along with the mutator
        let mut storage = MutatorInTLS {
            default_bump_pointer,
            mutator,
        };

        // Allocate
        let mut allocate_default = |size: usize| -> Address {
            // Alignment code is omitted here to make the code simpler to read.
            // In an actual implementation, alignment and offset need to be considered by the bindings.
            let new_cursor = storage.default_bump_pointer.cursor + size;
            if new_cursor < storage.default_bump_pointer.limit {
                let addr = storage.default_bump_pointer.cursor;
                storage.default_bump_pointer.cursor = new_cursor;
                addr
            } else {
                use crate::mmtk::util::alloc::Allocator;
                let selector = mmtk::memory_manager::get_allocator_mapping(
                    &fixture.mmtk,
                    AllocationSemantics::Default,
                );
                let default_allocator = unsafe {
                    storage
                        .mutator
                        .allocator_impl_mut::<mmtk::util::alloc::BumpAllocator<DummyVM>>(selector)
                };
                // Copy bump pointer values to the allocator in the mutator
                default_allocator.bump_pointer = storage.default_bump_pointer;
                // Do slow path allocation with MMTk
                let addr = default_allocator.alloc_slow(size, 8, 0);
                // Copy bump pointer values to the fastpath BumpPointer so we will have an allocation buffer.
                storage.default_bump_pointer = default_allocator.bump_pointer;
                addr
            }
        };

        // Allocate: this will fail in the fastpath, and will get an allocation buffer from the slowpath
        let addr1 = allocate_default(8);
        // Alloacte: this will allocate from the fastpath
        let addr2 = allocate_default(8);
        // ANCHOR_END: mutator_storage_embed_fastpath_struct

        assert!(!addr1.is_zero());
        assert!(!addr2.is_zero());
    })
}
