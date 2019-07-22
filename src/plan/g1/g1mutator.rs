use policy::regionspace::RegionSpace;
use policy::immortalspace::ImmortalSpace;
use util::alloc::{BumpAllocator, RegionAllocator};
use plan::mutator_context::MutatorContext;
use plan::Phase;
use plan::semispace;
use util::{Address, ObjectReference};
use util::alloc::Allocator;
use plan::Allocator as AllocationType;
use plan::plan;
use vm::{Collection, VMCollection};
use util::heap::{PageResource, MonotonePageResource};
use plan::g1::{PLAN, DEBUG};
use util::alloc::LargeObjectAllocator;
use policy::largeobjectspace::LargeObjectSpace;

use libc::c_void;

#[repr(C)]
pub struct G1Mutator {
    // CopyLocal
    rs: RegionAllocator,
    los: LargeObjectAllocator,
    vs: BumpAllocator<MonotonePageResource<ImmortalSpace>>,
}

impl MutatorContext for G1Mutator {
    fn collection_phase(&mut self, tls: *mut c_void, phase: &Phase, primary: bool) {
        if DEBUG {
            println!("Mutator {:?}", phase);
        }
        match phase {
            &Phase::PrepareStacks => {
                if !plan::stacks_prepared() {
                    VMCollection::prepare_mutator(self.rs.tls, self);
                }
                self.flush_remembered_sets();
            }
            &Phase::Prepare => {
                self.rs.reset();
            }
            &Phase::Release => {
                // rebind the allocation bump pointer to the appropriate semispace
                // self.rs.rebind(Some(semispace::PLAN.tospace()));
                self.rs.reset();
            }
            &Phase::EvacuatePrepare => {
                self.rs.reset();
            }
            &Phase::EvacuateRelease => {
                // rebind the allocation bump pointer to the appropriate semispace
                // self.rs.rebind(Some(semispace::PLAN.tospace()));
                self.rs.reset();
            }
            _ => {
                panic!("Per-mutator phase not handled!")
            }
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc({}, {}, {}, {:?})", size, align, offset, allocator);
        debug_assert!(self.rs.space as *const _ == &PLAN.region_space as *const _,
                      "regionallocator {:?} holds wrong space, rs.space: {:?}, region_space: {:?}",
                      self as *const _,
                      self.rs.get_space().unwrap() as *const _,
                      &PLAN.region_space as *const _);
        match allocator {
            AllocationType::Default => self.rs.alloc(size, align, offset),
            AllocationType::Los => self.los.alloc(size, align, offset),
            _ => self.vs.alloc(size, align, offset),
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc_slow({}, {}, {}, {:?})", size, align, offset, allocator);
        debug_assert!(self.rs.get_space().unwrap() as *const _ == &PLAN.region_space as *const _,
                      "regionallocator {:?} holds wrong space, rs.space: {:?}, region_space: {:?}",
                      self as *const _,
                      self.rs.get_space().unwrap() as *const _,
                      &PLAN.region_space as *const _);
        match allocator {
            AllocationType::Default => self.rs.alloc_slow(size, align, offset),
            AllocationType::Los => self.los.alloc(size, align, offset),
            _ => self.vs.alloc_slow(size, align, offset),
        }
    }

    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize, allocator: AllocationType) {
        debug_assert!(self.rs.get_space().unwrap() as *const _ == &PLAN.region_space as *const _);
        match allocator {
            AllocationType::Default => {}
            AllocationType::Los => {
                PLAN.los.initialize_header(refer, true);
            }
            _ => {
                // FIXME: data race on immortalspace.mark_state !!!
                let unsync = unsafe { &*PLAN.unsync.get() };
                unsync.versatile_space.initialize_header(refer);
            }
        }
    }

    fn get_tls(&self) -> *mut c_void {
        debug_assert!(self.rs.tls == self.vs.tls);
        self.rs.tls
    }
}

impl G1Mutator {
    pub fn new(tls: *mut c_void, space: &'static RegionSpace, los: &'static LargeObjectSpace, versatile_space: &'static ImmortalSpace) -> Self {
        G1Mutator {
            rs: RegionAllocator::new(tls, space),
            los: LargeObjectAllocator::new(tls, Some(los)),
            vs: BumpAllocator::new(tls, Some(versatile_space)),
        }
    }
}