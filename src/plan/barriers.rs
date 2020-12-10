use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::util::*;
use crate::util::constants::*;
use crate::util::heap::layout::vm_layout_constants::*;
use crate::MMTK;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashSet;

/// For field writes in HotSpot, we cannot always get the source object pointer and the field address
pub enum WriteTarget {
    Object(ObjectReference),
    Slot(Address),
}

pub trait Barrier: 'static + Send + Sync {
    fn flush(&mut self);
    fn post_write_barrier(&mut self, target: WriteTarget);
}

pub struct NoBarrier;

impl Barrier for NoBarrier {
    fn flush(&mut self) {}
    fn post_write_barrier(&mut self, _target: WriteTarget) {}
}

#[derive(Default)]
pub struct ModBuffer {
    modified_nodes: Vec<ObjectReference>,
    modified_edges: Vec<Address>,
}

pub struct FieldRememberingBarrier<E: ProcessEdgesWork, S: Space<E::VM>> {
    mmtk: &'static MMTK<E::VM>,
    nursery: &'static S,
    mod_buffer: ModBuffer,
}

pub struct BitRef {
    base: Address,
    word_offset: usize,
    bit_offset: usize,
}

impl BitRef {
    pub fn log_bit_of(addr: Address) -> Self {
        let base = conversions::metadata_start(addr);
        let word_index = (addr.as_usize() & (BYTES_IN_CHUNK - 1)) >> LOG_BYTES_IN_WORD;
        let word_offset = word_index >> LOG_BYTES_IN_WORD;
        let bit_offset = word_index & (BYTES_IN_WORD - 1);
        Self {
            base,
            word_offset,
            bit_offset
        }
    }

    pub fn attempt(&self, old: bool, new: bool) -> bool {
        let old_bit = if old { 0b1usize } else { 0b0usize };
        let new_bit = if new { 0b1usize } else { 0b0usize };
        let mask = 1 << self.bit_offset;
        let word = unsafe { &*((self.base.as_usize() + self.word_offset) as *const AtomicUsize) };
        loop {
            let old = word.load(Ordering::SeqCst);
            if ((old & mask) >> self.bit_offset) != old_bit {
                return false;
            }
            let new = (old & !mask) | (new_bit << self.bit_offset);
            if old == word.compare_and_swap(old, new, Ordering::SeqCst) {
                return true;
            }
        }
    }
}

impl<E: ProcessEdgesWork, S: Space<E::VM>> FieldRememberingBarrier<E, S> {
    #[allow(unused)]
    pub fn new(mmtk: &'static MMTK<E::VM>, nursery: &'static S) -> Self {
        Self {
            mmtk,
            nursery,
            mod_buffer: ModBuffer::default(),
        }
    }

    fn enqueue_node(&mut self, obj: ObjectReference) {
        if BitRef::log_bit_of(obj.to_address()).attempt(false, true) {
            self.mod_buffer.modified_nodes.push(obj);
            if self.mod_buffer.modified_nodes.len() >= E::CAPACITY {
                self.flush();
            }
        }
    }

    fn enqueue_edge(&mut self, slot: Address) {
        if BitRef::log_bit_of(slot).attempt(false, true) {
            self.mod_buffer.modified_edges.push(slot);
            if self.mod_buffer.modified_edges.len() >= 512 {
                self.flush();
            }
        }
    }
}

impl<E: ProcessEdgesWork, S: Space<E::VM>> Barrier for FieldRememberingBarrier<E, S> {
    fn flush(&mut self) {
        let mut modified_nodes = vec![];
        std::mem::swap(&mut modified_nodes, &mut self.mod_buffer.modified_nodes);
        let mut modified_edges = vec![];
        std::mem::swap(&mut modified_edges, &mut self.mod_buffer.modified_edges);
        debug_assert!(
            !self.mmtk.scheduler.final_stage.is_activated(),
            "{:?}",
            self as *const _
        );
        self.mmtk
            .scheduler
            .closure_stage
            .add(ProcessModBuf::<E>::new(modified_nodes, modified_edges));
    }
    fn post_write_barrier(&mut self, target: WriteTarget) {
        match target {
            WriteTarget::Object(obj) => {
                if !self.nursery.in_space(obj) {
                    self.enqueue_node(obj);
                }
            }
            WriteTarget::Slot(slot) => {
                if !self.nursery.address_in_space(slot) {
                    self.enqueue_edge(slot);
                }
            }
        }
    }
}
