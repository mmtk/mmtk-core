use crate::policy::space::Space;

use super::SSMutator;
use super::SSTraceLocal;
use super::SSCollector;

use crate::plan::plan;
use crate::plan::Plan;
use crate::plan::Allocator;
use crate::policy::copyspace::CopySpace;
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::plan::Phase;
use crate::plan::trace::Trace;
use crate::util::ObjectReference;
use crate::util::heap::layout::Mmapper as IMmapper;
use crate::util::Address;
use crate::util::heap::VMRequest;
use crate::util::OpaquePointer;

use std::cell::UnsafeCell;
use std::sync::atomic::{self, Ordering};

use crate::vm::Scanning;
use crate::util::conversions::bytes_to_pages;
use crate::plan::plan::{create_vm_space, CommonPlan};
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::options::UnsafeOptionsWrapper;
use std::sync::Arc;
use crate::util::heap::HeapMeta;
use crate::util::heap::layout::vm_layout_constants::{HEAP_START, HEAP_END};
use crate::vm::VMBinding;

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
    pub vm_space: Option<ImmortalSpace<VM>>,
    pub copyspace0: CopySpace<VM>,
    pub copyspace1: CopySpace<VM>,
    pub versatile_space: ImmortalSpace<VM>,
    pub los: LargeObjectSpace<VM>,
}

unsafe impl<VM: VMBinding> Sync for SemiSpace<VM> {}

impl<VM: VMBinding> Plan<VM> for SemiSpace<VM> {
    type MutatorT = SSMutator<VM>;
    type TraceLocalT = SSTraceLocal<VM>;
    type CollectorT = SSCollector<VM>;

    fn new(vm_map: &'static VMMap, mmapper: &'static Mmapper, options: Arc<UnsafeOptionsWrapper>) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        SemiSpace {
            unsync: UnsafeCell::new(SemiSpaceUnsync {
                hi: false,
                vm_space: if options.vm_space {
                    Some(create_vm_space(vm_map, mmapper, &mut heap, options.vm_space_size))
                } else {
                    None
                },
                copyspace0: CopySpace::new("copyspace0", false, true,
                                           VMRequest::discontiguous(), vm_map, mmapper, &mut heap),
                copyspace1: CopySpace::new("copyspace1", true, true,
                                           VMRequest::discontiguous(), vm_map, mmapper, &mut heap),
                versatile_space: ImmortalSpace::new("versatile_space", true,
                                                    VMRequest::discontiguous(), vm_map, mmapper, &mut heap),
                los: LargeObjectSpace::new("los", true, VMRequest::discontiguous(), vm_map, mmapper, &mut heap),
            }),
            ss_trace: Trace::new(),
            common: CommonPlan::new(vm_map, mmapper, options, heap),
        }
    }

    fn gc_init(&self, heap_size: usize, vm_map: &'static VMMap) {
        vm_map.finalize_static_space_map(self.common.heap.get_discontig_start(), self.common.heap.get_discontig_end());

        let unsync = unsafe { &mut *self.unsync.get() };
        self.common.heap.total_pages.store(bytes_to_pages(heap_size), Ordering::Relaxed);
        if unsync.vm_space.is_some() {
            unsync.vm_space.as_mut().unwrap().init(vm_map);
        }
        unsync.copyspace0.init(vm_map);
        unsync.copyspace1.init(vm_map);
        unsync.versatile_space.init(vm_map);
        unsync.los.init(vm_map);
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn bind_mutator(&'static self, tls: OpaquePointer) -> Box<SSMutator<VM>> {
        Box::new(SSMutator::new(tls, self))
    }

    fn will_never_move(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };

        if self.tospace().in_space(object) || self.fromspace().in_space(object) {
            return false;
        }

        if unsync.versatile_space.in_space(object) || unsync.los.in_space(object) {
            return true;
        }

        // this preserves correctness over efficiency
        false
    }

    fn is_valid_ref(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.versatile_space.in_space(object) {
            return true;
        }
        if unsync.vm_space.is_some() && unsync.vm_space.as_ref().unwrap().in_space(object) {
            return true;
        }
        if self.tospace().in_space(object) {
            return true;
        }
        if unsync.los.in_space(object) {
            return true;
        }
        false
    }

    unsafe fn collection_phase(&self, tls: OpaquePointer, phase: &Phase) {
        let unsync = &mut *self.unsync.get();

        match phase {
            Phase::SetCollectionKind => {
                self.common.cur_collection_attempts.store(if self.is_user_triggered_collection() {
                    1
                } else {
                    self.determine_collection_attempts()
                }, Ordering::Relaxed);

                let emergency_collection = !self.is_internal_triggered_collection()
                    && self.last_collection_was_exhaustive() && self.common.cur_collection_attempts.load(Ordering::Relaxed) > 1;
                self.common().emergency_collection.store(emergency_collection, Ordering::Relaxed);

                if emergency_collection {
                    self.force_full_heap_collection();
                }
            }
            Phase::Initiate => {
                self.common.set_gc_status(plan::GcStatus::GcPrepare);
            }
            Phase::PrepareStacks => {
                self.common.stacks_prepared.store(true, atomic::Ordering::SeqCst);
            }
            Phase::Prepare => {
                #[cfg(feature = "sanity")]
                {
                    use crate::util::sanity::sanity_checker::SanityChecker;
                    println!("Pre GC sanity check");
                    SanityChecker::new(tls, &self).check();
                }
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
                unsync.versatile_space.prepare();
                if unsync.vm_space.is_some() {
                    unsync.vm_space.as_mut().unwrap().prepare();
                }
                unsync.los.prepare(true);
            }
            &Phase::StackRoots => {
                VM::VMScanning::notify_initial_thread_scan_complete(false, tls);
                self.common.set_gc_status(plan::GcStatus::GcProper);
            }
            &Phase::Roots => {
                VM::VMScanning::reset_thread_counter();
                self.common.set_gc_status(plan::GcStatus::GcProper);
            }
            &Phase::Closure => {}
            &Phase::Release => {
                #[cfg(feature = "sanity")]
                {
                    use crate::util::constants::LOG_BYTES_IN_PAGE;
                    use libc::memset;
                    use crate::util::heap::PageResource;
                    if self.fromspace().common().contiguous {
                        let fromspace_start = self.fromspace().common().start;
                        let fromspace_commited = self.fromspace().common().pr.as_ref().unwrap().common().committed.load(Ordering::Relaxed);
                        let commited_bytes = fromspace_commited * (1 << LOG_BYTES_IN_PAGE);
                        println!("Destroying fromspace {}~{}", fromspace_start, fromspace_start + commited_bytes);
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
                unsync.versatile_space.release();
                if unsync.vm_space.is_some() {
                    unsync.vm_space.as_mut().unwrap().release();
                }
                unsync.los.release(true);
            }
            Phase::Complete => {
                #[cfg(feature = "sanity")]
                {
                    use crate::util::sanity::sanity_checker::SanityChecker;
                    use crate::util::sanity::memory_scan;
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

                self.common.set_gc_status(plan::GcStatus::NotInGC);
            }
            _ => {
                panic!("Global phase not handled!")
            }
        }
    }

    fn get_collection_reserve(&self) -> usize {
        self.tospace().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        let unsync = unsafe{&*self.unsync.get()};
        self.tospace().reserved_pages() + unsync.versatile_space.reserved_pages() + unsync.los.reserved_pages()
    }

    fn is_bad_ref(&self, object: ObjectReference) -> bool {
        self.fromspace().in_space(object)
    }

    fn is_movable(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.vm_space.is_some() && unsync.vm_space.as_ref().unwrap().in_space(object) {
            return unsync.vm_space.as_ref().unwrap().is_movable();
        }
        if unsync.copyspace0.in_space(object) {
            return unsync.copyspace0.is_movable();
        }
        if unsync.copyspace1.in_space(object) {
            return unsync.copyspace1.is_movable();
        }
        if unsync.versatile_space.in_space(object) {
            return unsync.versatile_space.is_movable();
        }
        if unsync.los.in_space(object) {
            return unsync.los.is_movable();
        }
        true
    }

    fn is_mapped_address(&self, address: Address) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsafe{
            (unsync.vm_space.is_some() && unsync.vm_space.as_ref().unwrap().in_space(address.to_object_reference()))  ||
            unsync.versatile_space.in_space(address.to_object_reference()) ||
            unsync.copyspace0.in_space(address.to_object_reference()) ||
            unsync.copyspace1.in_space(address.to_object_reference()) ||
            unsync.los.in_space(address.to_object_reference())
        } {
            self.common.mmapper.address_is_mapped(address)
        } else {
            false
        }
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

    pub fn get_versatile_space(&self) -> &'static ImmortalSpace<VM> {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.versatile_space
    }

    pub fn get_los(&self) -> &'static LargeObjectSpace<VM> {
        let unsync = unsafe { &*self.unsync.get() };

        &unsync.los
    }
}