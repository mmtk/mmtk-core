use crate::plan::global::CommonPlan;
use crate::plan::selected_plan::SelectedPlan;
use crate::plan::Allocator as AllocationType;
use crate::plan::Phase;
use crate::util::alloc::{Allocator, BumpAllocator, LargeObjectAllocator};
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;

pub trait MutatorContext<VM: VMBinding>: Send + Sync + 'static {
    fn common(&self) -> &CommonMutatorContext<VM>;
    fn prepare(&mut self, tls: OpaquePointer);
    fn release(&mut self, tls: OpaquePointer);
    fn alloc(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        allocator: AllocationType,
    ) -> Address;
    fn post_alloc(
        &mut self,
        refer: ObjectReference,
        type_refer: ObjectReference,
        bytes: usize,
        allocator: AllocationType,
    );
    fn flush_remembered_sets(&mut self) {}
    fn flush(&mut self) {
        self.flush_remembered_sets();
    }
    fn get_tls(&self) -> OpaquePointer;

    fn object_reference_write(&mut self, src: ObjectReference, slot: Address, value: ObjectReference) {}
    fn record_modified_node(&mut self, obj: ObjectReference) {}
    fn record_modified_edge(&mut self, slot: Address) {}
}

pub struct CommonMutatorContext<VM: VMBinding> {
    immortal: BumpAllocator<VM>,
    los: LargeObjectAllocator<VM>,
}

impl<VM: VMBinding> CommonMutatorContext<VM> {
    pub fn new(
        tls: OpaquePointer,
        plan: &'static SelectedPlan<VM>,
        common_plan: &'static CommonPlan<VM>,
    ) -> Self {
        CommonMutatorContext {
            immortal: BumpAllocator::new(tls, Some(common_plan.get_immortal()), plan),
            los: LargeObjectAllocator::new(tls, Some(common_plan.get_los()), plan),
        }
    }

    pub fn alloc(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        allocator: AllocationType,
    ) -> Address {
        match allocator {
            AllocationType::Los => self.los.alloc(size, align, offset),
            AllocationType::Immortal => self.immortal.alloc(size, align, offset),
            _ => panic!("Unexpected allocator for alloc(): {:?}", allocator),
        }
    }

    pub fn post_alloc(
        &mut self,
        object: ObjectReference,
        _type: ObjectReference,
        _bytes: usize,
        allocator: AllocationType,
    ) {
        match allocator {
            AllocationType::Los => {
                self.los
                    .get_space()
                    .unwrap()
                    .initialize_header(object, true);
            }
            AllocationType::Immortal => self
                .immortal
                .get_space()
                .unwrap()
                .initialize_header(object, true),
            _ => panic!("Unexpected allocator for post_alloc(): {:?}", allocator),
        }
    }
}
