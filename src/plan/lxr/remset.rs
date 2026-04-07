use std::{cell::UnsafeCell, marker::PhantomData};

use crate::policy::immix::ImmixSpace;
use crate::policy::space::Space;
use crate::util::metadata::side_metadata::spec_defs::{IX_LINE_REUSE_COUNT, LOS_PAGE_REUSE_COUNT};
use crate::util::ObjectReference;
use crate::{
    plan::lxr::LXR,
    scheduler::{GCWork, GCWorker},
    util::Address,
    vm::{slot::Slot, VMBinding},
    MMTK,
};

use super::mature_evac::EvacuateMatureObjects;
use atomic::Ordering;
use std::sync::atomic::AtomicUsize;

#[repr(C)]
pub(super) struct RemSetEntry(Address, u8);

impl RemSetEntry {
    fn encode<VM: VMBinding>(slot: VM::VMSlot, ix: bool) -> Self {
        let reuse = if ix {
            IX_LINE_REUSE_COUNT.load_atomic::<u8>(slot.to_address(), Ordering::SeqCst)
        } else {
            LOS_PAGE_REUSE_COUNT.load_atomic::<u8>(slot.to_address(), Ordering::SeqCst)
        };
        Self(slot.raw_address(), reuse)
    }

    pub fn decode<VM: VMBinding>(&self) -> (VM::VMSlot, u8) {
        (VM::VMSlot::from_address(self.0), self.1)
    }
}

pub struct MatureEvecRemSet<VM: VMBinding> {
    pub(super) gc_buffers: Vec<UnsafeCell<Vec<RemSetEntry>>>,
    local_packets: Vec<UnsafeCell<Vec<Box<dyn GCWork<VM>>>>>,
    _p: PhantomData<VM>,
    size: AtomicUsize,
}

impl<VM: VMBinding> MatureEvecRemSet<VM> {
    pub fn new(workers: usize) -> Self {
        let mut rs = Self {
            gc_buffers: vec![],
            local_packets: vec![],
            _p: PhantomData,
            size: AtomicUsize::new(0),
        };
        rs.gc_buffers
            .resize_with(workers, || UnsafeCell::new(vec![]));
        rs.local_packets
            .resize_with(workers, || UnsafeCell::new(vec![]));
        rs
    }

    fn gc_buffer(&self, id: usize) -> &mut Vec<RemSetEntry> {
        unsafe { &mut *self.gc_buffers[id].get() }
    }

    fn flush_all(&self, space: &ImmixSpace<VM>) {
        let mut mature_evac_remsets = space.mature_evac_remsets.lock().unwrap();
        self.size.store(0, Ordering::SeqCst);
        for id in 0..self.gc_buffers.len() {
            if self.gc_buffer(id).len() > 0 {
                let remset = std::mem::take(self.gc_buffer(id));
                mature_evac_remsets.push(Box::new(EvacuateMatureObjects::new(remset)));
            }
        }
        for id in 0..self.local_packets.len() {
            let buf = unsafe { &mut *self.local_packets[id].get() };
            if buf.len() > 0 {
                let packets = std::mem::take(buf);
                for p in packets {
                    mature_evac_remsets.push(p);
                }
            }
        }
    }

    #[cold]
    fn flush(&self, id: usize) {
        if self.gc_buffer(id).len() > 0 {
            let remset = std::mem::take(self.gc_buffer(id));
            self.size.fetch_add(remset.len(), Ordering::SeqCst);
            let w = EvacuateMatureObjects::new(remset);
            let packet_buffer = unsafe { &mut *self.local_packets[id].get() };
            packet_buffer.push(Box::new(w));
        }
    }

    pub fn record(&self, s: VM::VMSlot, _o: ObjectReference, lxr: &LXR<VM>) {
        let id = crate::scheduler::current_worker_ordinal().unwrap();
        let ix = lxr.immix_space.address_in_space(s.to_address());
        self.gc_buffer(id).push(RemSetEntry::encode::<VM>(s, ix));
        if self.gc_buffer(id).len() >= EvacuateMatureObjects::<VM>::CAPACITY {
            self.flush(id)
        }
    }
}

pub struct FlushMatureEvacRemsets;

impl<VM: VMBinding> GCWork<VM> for FlushMatureEvacRemsets {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let immix_space = &mmtk
            .get_plan()
            .downcast_ref::<LXR<VM>>()
            .unwrap()
            .immix_space;
        immix_space.mature_evac_remset.flush_all(immix_space);
        immix_space.process_mature_evacuation_remset();
    }
}
