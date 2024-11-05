//! This is a simple module to log slogs and check for duplicate slots.
//!
//! It uses a hash-set to keep track of slots, and is so very expensive.
//! We currently only use this as part of the `extreme_assertions` feature.
//!

use crate::plan::Plan;
use crate::util::log;
use crate::vm::slot::Slot;
use crate::vm::VMBinding;
use std::collections::HashSet;
use std::sync::RwLock;

pub struct SlotLogger<SL: Slot> {
    // A private hash-set to keep track of slots.
    slot_log: RwLock<HashSet<SL>>,
}

unsafe impl<SL: Slot> Sync for SlotLogger<SL> {}

impl<SL: Slot> SlotLogger<SL> {
    pub fn new() -> Self {
        Self {
            slot_log: Default::default(),
        }
    }

    /// Logs a slot.
    /// Panics if the slot was already logged.
    ///
    /// # Arguments
    ///
    /// * `slot` - The slot to log.
    ///
    pub fn log_slot(&self, slot: SL) {
        log::trace!("log_slot({:?})", slot);
        let mut slot_log = self.slot_log.write().unwrap();
        assert!(
            slot_log.insert(slot),
            "duplicate slot ({:?}) detected",
            slot
        );
    }

    /// Reset the slot logger by clearing the hash-set of slots.
    /// This function is called at the end of each GC iteration.
    ///
    pub fn reset(&self) {
        let mut slot_log = self.slot_log.write().unwrap();
        slot_log.clear();
    }
}

/// Whether we should check duplicate slots. This depends on the actual plan.
pub fn should_check_duplicate_slots<VM: VMBinding>(plan: &dyn Plan<VM = VM>) -> bool {
    // If a plan allows tracing duplicate edges, we should not run this check.
    !plan.constraints().may_trace_duplicate_edges
}
