use crate::policy::space::Space;

use super::SSCollector;
use super::SSMutator;
use super::SSTraceLocal;

use crate::plan::trace::Trace;
use crate::plan::Allocator;
use crate::plan::Phase;
use crate::plan::Plan;
use crate::policy::copyspace::CopySpace;
use crate::util::heap::VMRequest;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::OpaquePointer;

use std::cell::UnsafeCell;
#[cfg(feature = "sanity")]
use std::sync::atomic::Ordering;

use crate::plan::plan::BasePlan;
use crate::plan::plan::CommonPlan;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
use crate::vm::VMBinding;
use std::sync::Arc;

pub type SelectedPlan<VM> = SemiSpace<VM>;

pub const ALLOC_SS: Allocator = Allocator::Default;
pub const SCAN_BOOT_IMAGE: bool = true;

pub struct SemiSpace<VM: VMBinding> {
    pub unsync: UnsafeCell<SemiSpaceUnsync<VM>>,
    pub ss_trace: Trace,
    pub common: CommonPlan<VM>,
}

pub struct SemiSpaceUnsync<VM: VMBinding> {
    pub hi: bool,
    pub copyspace0: CopySpace<VM>,
    pub copyspace1: CopySpace<VM>,
}

unsafe impl<VM: VMBinding> Sync for SemiSpace<VM> {}

impl<VM: VMBinding> Plan<VM> for SemiSpace<VM> {
    type MutatorT = SSMutator<VM>;
    type TraceLocalT = SSTraceLocal<VM>;
    type CollectorT = SSCollector<VM>;

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        SemiSpace {
            unsync: UnsafeCell::new(SemiSpaceUnsync {
                hi: false,
                copyspace0: CopySpace::new(
                    "copyspace0",
                    false,
                    true,
                    VMRequest::discontiguous(),
                    vm_map,
                    mmapper,
                    &mut heap,
                ),
                copyspace1: CopySpace::new(
                    "copyspace1",
                    true,
                    true,
                    VMRequest::discontiguous(),
                    vm_map,
                    mmapper,
                    &mut heap,
                ),
            }),
            ss_trace: Trace::new(),
            common: CommonPlan::new(vm_map, mmapper, options, heap),
        }
    }

    fn gc_init(&self, heap_size: usize, vm_map: &'static VMMap) {
        self.common.gc_init(heap_size, vm_map);

        let unsync = unsafe { &mut *self.unsync.get() };
        unsync.copyspace0.init(vm_map);
        unsync.copyspace1.init(vm_map);
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn bind_mutator(&'static self, tls: OpaquePointer) -> Box<SSMutator<VM>> {
        Box::new(SSMutator::new(tls, self))
    }

    fn is_valid_ref(&self, object: ObjectReference) -> bool {
        if self.tospace().in_space(object) {
            return true;
        }
        self.common.is_valid_ref(object)
    }

    unsafe fn collection_phase(&self, tls: OpaquePointer, phase: &Phase) {
        let unsync = &mut *self.unsync.get();
        match phase {
            Phase::Prepare => {
                self.common.collection_phase(tls, phase, true);
                debug_assert!(self.ss_trace.values.is_empty());
                debug_assert!(self.ss_trace.root_locations.is_empty());
                #[cfg(feature = "sanity")]
                {
                    self.fromspace().unprotect();
                }
                unsync.hi = !unsync.hi; // flip the semi-spaces
                                        // prepare each of the collected regions
                unsync.copyspace0.prepare(unsync.hi);
                unsync.copyspace1.prepare(!unsync.hi);

                #[cfg(feature = "sanity")]
                {
                    use crate::util::sanity::sanity_checker::SanityChecker;
                    println!("Pre GC sanity check");
                    SanityChecker::new(tls, &self).check();
                }
            }
            &Phase::Release => {
                self.common.collection_phase(tls, phase, true);
                #[cfg(feature = "sanity")]
                {
                    use crate::util::constants::LOG_BYTES_IN_PAGE;
                    use crate::util::heap::PageResource;
                    use libc::memset;
                    if self.fromspace().common().contiguous {
                        let fromspace_start = self.fromspace().common().start;
                        let fromspace_commited = self
                            .fromspace()
                            .common()
                            .pr
                            .as_ref()
                            .unwrap()
                            .common()
                            .committed
                            .load(Ordering::Relaxed);
                        let commited_bytes = fromspace_commited * (1 << LOG_BYTES_IN_PAGE);
                        println!(
                            "Destroying fromspace {}~{}",
                            fromspace_start,
                            fromspace_start + commited_bytes
                        );
                        memset(fromspace_start.to_mut_ptr(), 0xFF, commited_bytes);
                    } else {
                        println!("Fromspace is discontiguous, not destroying")
                    }
                }
                // release the collected region
                if unsync.hi {
                    unsync.copyspace0.release();
                } else {
                    unsync.copyspace1.release();
                }
            }
            Phase::Complete => {
                #[cfg(feature = "sanity")]
                {
                    use crate::util::sanity::memory_scan;
                    use crate::util::sanity::sanity_checker::SanityChecker;
                    println!("Post GC sanity check");
                    SanityChecker::new(tls, &self).check();
                    println!("Post GC memory scan");
                    memory_scan::scan_region(&self);
                    println!("Finished one GC");
                }
                debug_assert!(self.ss_trace.values.is_empty());
                debug_assert!(self.ss_trace.root_locations.is_empty());
                #[cfg(feature = "sanity")]
                {
                    self.fromspace().protect();
                }
                self.common.collection_phase(tls, phase, true);
            }
            _ => self.common.collection_phase(tls, phase, true),
        }
    }

    fn get_collection_reserve(&self) -> usize {
        self.tospace().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        self.tospace().reserved_pages() + self.common.get_pages_used()
    }

    fn is_bad_ref(&self, object: ObjectReference) -> bool {
        self.fromspace().in_space(object)
    }

    fn is_in_space(&self, address: Address) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        let addr = unsafe { address.to_object_reference() };
        if unsync.copyspace0.in_space(addr) || unsync.copyspace1.in_space(addr) {
            return true;
        }
        self.common.in_common_space(addr)
    }
}

impl<VM: VMBinding> SemiSpace<VM> {
    pub fn tospace(&self) -> &'static CopySpace<VM> {
        let unsync = unsafe { &*self.unsync.get() };

        if unsync.hi {
            &unsync.copyspace1
        } else {
            &unsync.copyspace0
        }
    }

    pub fn fromspace(&self) -> &'static CopySpace<VM> {
        let unsync = unsafe { &*self.unsync.get() };

        if unsync.hi {
            &unsync.copyspace0
        } else {
            &unsync.copyspace1
        }
    }

    pub fn get_sstrace(&self) -> &Trace {
        &self.ss_trace
    }
}
