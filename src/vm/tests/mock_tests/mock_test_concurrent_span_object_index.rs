// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! Object index lookup (design §5.13): for every size class, fill a span and check that every
//! 8-byte offset of every object maps back to that object through the page map and the
//! `div_magic` divide, and that tracing an object marks exactly that object.

use super::concurrent_span_util::*;
use super::mock_test_prelude::*;
use crate::policy::spanspace::sizeclasses::SIZE_CLASS_TO_SIZE;
use crate::util::ObjectReference;

#[test]
pub fn object_index_at_every_offset() {
    with_mockvm(
        default_setup,
        || {
            let fixture = MutatorFixture::create_with_heapsize(64 << 20);
            let Some(space) = span_space(fixture.mmtk()) else {
                eprintln!("No SpanSpace in this plan: test skipped");
                return;
            };
            let mutator = fixture.mutator();

            let mut all: Vec<(ObjectReference, usize)> = vec![];
            // Class 1 (8 bytes) is skipped: MockVM object references point 8 bytes into the
            // object, so MockVM objects are at least 16 bytes.
            for (class, &size) in SIZE_CLASS_TO_SIZE.iter().enumerate().skip(2) {
                let size = size as usize;
                // Fill the first span of this class.
                let first = alloc_object(mutator, size);
                let span = space.span_of(object_start(first)).unwrap();
                assert_eq!(span.elemsize(), size, "class {class}");
                let mut objects = vec![first];
                while objects.len() < span.nelems() {
                    objects.push(alloc_object(mutator, size));
                }
                for (i, obj) in objects.iter().enumerate() {
                    let start = object_start(*obj);
                    assert_eq!(start, span.start() + i * size, "class {class} object {i}");
                    for off in (0..size).step_by(8) {
                        let s = space.span_of(start + off).unwrap();
                        assert!(std::ptr::eq(s, span));
                        assert_eq!(s.obj_index(start + off), i, "class {class} off {off}");
                    }
                    all.push((*obj, size));
                }
            }

            // Tracing marks exactly the traced object.
            space.prepare_initial_mark();
            for (i, (obj, _)) in all.iter().enumerate() {
                assert!(!obj.is_live());
                mark(space, &all[i..i + 1]);
                assert!(obj.is_live());
                if let Some((next, _)) = all.get(i + 1) {
                    assert!(!next.is_live());
                }
            }
            let expected: usize = all.iter().map(|(_, s)| *s).sum();
            assert_eq!(space.bytes_marked(), expected);
            space.release_after_mark();
            release_span_allocator(mutator);
            sweep_all(space);
            for (obj, _) in all.iter() {
                assert!(obj.is_live());
            }
        },
        no_cleanup,
    )
}
