use super::worker::*;
use crate::vm::VMBinding;
use crate::mmtk::MMTK;
use super::*;



pub trait Work<C: Context>: 'static + Send + Sync {
    fn do_work(&mut self, worker: &'static mut Worker<C>, context: &'static C);
}

impl <VM: VMBinding> PartialEq for Box<dyn Work<VM>> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() as *const dyn Work<VM> == other.as_ref() as *const dyn Work<VM>
    }
}

impl <VM: VMBinding> Eq for Box<dyn Work<VM>> {}

pub trait GCWork<VM: VMBinding>: 'static + Send + Sync + Sized + Work<MMTK<VM>> {
    fn do_work(&mut self, worker: &'static mut GCWorker<VM>, mmtk: &'static MMTK<VM>);
}

impl <VM: VMBinding, W: GCWork<VM>> Work<MMTK<VM>> for W {
    default fn do_work(&mut self, worker: &'static mut Worker<MMTK<VM>>, mmtk: &'static MMTK<VM>) {
        GCWork::do_work(self, worker, mmtk)
    }
}
