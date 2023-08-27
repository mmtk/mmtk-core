use crate::DummyVM;
use crate::SINGLETON;
use mmtk::util::opaque_pointer::*;
use mmtk::vm::ActivePlan;
use mmtk::Mutator;
use mmtk::Plan;

pub struct VMActivePlan {}

impl ActivePlan<DummyVM> for VMActivePlan {
    fn global() -> &'static dyn Plan<VM = DummyVM> {
        SINGLETON.get_plan()
    }

    fn number_of_mutators() -> usize {
        unimplemented!()
    }

    fn is_mutator(_tls: VMThread) -> bool {
        // FIXME
        true
    }

    fn mutator(_tls: VMMutatorThread) -> &'static mut Mutator<DummyVM> {
        unimplemented!()
    }

    fn mutators<'a>() -> Box<dyn Iterator<Item = &'a mut Mutator<DummyVM>> + 'a> {
        unimplemented!()
    }
}
