use atomic::Ordering;

use crate::plan::global::NoCopy;
use crate::plan::global::Plan;
use crate::policy::mallocspace::MallocSpace;
use crate::policy::mallocspace::metadata::is_chunk_mapped;
use crate::policy::mallocspace::metadata::is_chunk_marked_unsafe;
use crate::policy::space::Space;
use crate::scheduler::GCWork;
use crate::scheduler::GCWorker;
use crate::scheduler::WorkBucketStage;
use crate::scheduler::gc_work::*;
use crate::util::Address;
use crate::util::ObjectReference;
#[cfg(feature="malloc")]
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

use super::MarkSweep;

pub struct MSProcessEdges<VM: VMBinding> {
    plan: &'static MarkSweep<VM>,
    base: ProcessEdgesBase<MSProcessEdges<VM>>,
}

impl<VM: VMBinding> ProcessEdgesWork for MSProcessEdges<VM> {
    type VM = VM;

    const OVERWRITE_REFERENCE: bool = false;
    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<MarkSweep<VM>>().unwrap();
        Self { plan, base }
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        trace!("Tracing object {}", object);
        if self.plan.ms_space().in_space(object) {
            self.plan.ms_space().trace_object::<Self>(self, object)
        } else {
            self.plan
                .common()
                .trace_object::<Self, NoCopy<VM>>(self, object)
        }
    }
}

impl<VM: VMBinding> Deref for MSProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for MSProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}





/// Simple work packet that just sweeps a single chunk
#[cfg(feature="malloc")]
pub struct MSSweepChunk<VM: VMBinding> {
    ms: &'static MallocSpace<VM>,
    // starting address of a chunk
    chunk: Address,
}

#[cfg(feature="malloc")]
impl<VM: VMBinding> GCWork<VM> for MSSweepChunk<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.ms.sweep_chunk(self.chunk);
    }
}

/// Work packet that generates sweep jobs for gc workers. Each chunk is given its own work packet
#[cfg(feature="malloc")]
pub struct MSSweepChunks<VM: VMBinding> {
    plan: &'static MarkSweep<VM>,
}

#[cfg(feature="malloc")]
impl<VM: VMBinding> MSSweepChunks<VM> {
    pub fn new(plan: &'static MarkSweep<VM>) -> Self {
        Self { plan }
    }
}

#[cfg(feature="malloc")]
impl<VM: VMBinding> GCWork<VM> for MSSweepChunks<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let ms = self.plan.ms_space();
        let mut work_packets: Vec<Box<dyn GCWork<VM>>> = vec![];
        let mut chunk = unsafe { Address::from_usize(ms.chunk_addr_min.load(Ordering::Relaxed)) }; // XXX: have to use AtomicUsize to represent an Address
        let end = unsafe { Address::from_usize(ms.chunk_addr_max.load(Ordering::Relaxed)) }
            + BYTES_IN_CHUNK;

        // Since only a single thread generates the sweep work packets as well as it is a Stop-the-World collector,
        // we can assume that the chunk mark metadata is not being accessed by anything else and hence we use
        // non-atomic accesses
        while chunk < end {
            if is_chunk_mapped(chunk) && unsafe { is_chunk_marked_unsafe(chunk) } {
                work_packets.push(box MSSweepChunk { ms, chunk });
            }

            chunk += BYTES_IN_CHUNK;
        }

        debug!("Generated {} sweep work packets", work_packets.len());
        #[cfg(debug_assertions)]
        {
            ms.total_work_packets
                .store(work_packets.len() as u32, Ordering::SeqCst);
            ms.completed_work_packets.store(0, Ordering::SeqCst);
            ms.work_live_bytes.store(0, Ordering::SeqCst);
        }

        mmtk.scheduler.work_buckets[WorkBucketStage::Release].bulk_add(work_packets);
    }
}

pub struct MSGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MSGCWorkContext<VM> {
    type VM = VM;
    type PlanType = MarkSweep<VM>;
    type CopyContextType = NoCopy<VM>;
    type ProcessEdgesWorkType = MSProcessEdges<VM>;
}
