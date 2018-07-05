use std::sync::Mutex;
use std::cell::UnsafeCell;
use std::vec::Vec;

use ::util::Address;

// Debug flags
const TRACE: bool = false;
const TRACE_UNREACHABLE: bool = false;
const TRACE_DETAIL: bool = false;
const TRACE_FORWARD: bool = false;

// XXX: We differ from the original implementation
//      by ignoring "stress," i.e. where the array
//      of references is grown by 1 each time. We
//      can't do this here b/c std::vec::Vec doesn't
//      allow us to customize its behaviour like that.
//      (Similarly, GROWTH_FACTOR is locked at 2.0, but
//      luckily this is also the value used by Java MMTk.)
const INITIAL_SIZE: usize = 256;

pub struct ReferenceProcessor {
    // XXX: To support the possibility of the collector working
    //      on the reference in parallel, we wrap the structure
    //      in an UnsafeCell.+
    sync: UnsafeCell<Mutex<ReferenceprocessorSync>>,
}

pub enum Semantics {
    SOFT,
    WEAK,
    PHANTOM,
}

struct ReferenceprocessorSync {
    // XXX: A data race on any of these fields is UB. If
    //      parallelizing this code, change the types to
    //      have the correct semantics.
    /**
     * The table of reference objects for the current semantics
     */
    references: Vec<Address>,

    /*
     * In a MarkCompact (or similar) collector, we need to update the {@code references}
     * field, and then update its contents.  We implement this by saving the pointer in
     * this untraced field for use during the {@code forward} pass.
     */
    //unforwarded_references: Vec<Address>,
    // XXX: ^ Necessary?

    /**
     * Index into the <code>references</code> table for the start of
     * the reference nursery.
     */
    nursery_index: usize,

    /**
     * Index of the first free slot in the reference table.
     */
    max_index: usize,

    /**
     * Semantics
     */
    semantics: Semantics,
}

impl ReferenceProcessor {
    pub fn new(semantics: Semantics) -> Self {
        ReferenceProcessor {
            sync: UnsafeCell::new(Mutex::new(ReferenceprocessorSync {
                references: Vec::with_capacity(INITIAL_SIZE),
                nursery_index: 0,
                max_index: 0,
                semantics,
            })),
        }
    }
}