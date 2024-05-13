//! This is a simple module to log edges and check for duplicate edges.
//!
//! It uses a hash-set to keep track of edge, and is so very expensive.
//! We currently only use this as part of the `extreme_assertions` feature.
//!

use crate::plan::Plan;
use crate::vm::slot::Slot;
use crate::vm::VMBinding;
use std::collections::HashSet;
use std::sync::RwLock;

pub struct SlotLogger<SL: Slot> {
    // A private hash-set to keep track of edges.
    edge_log: RwLock<HashSet<SL>>,
}

unsafe impl<SL: Slot> Sync for SlotLogger<SL> {}

impl<SL: Slot> SlotLogger<SL> {
    pub fn new() -> Self {
        Self {
            edge_log: Default::default(),
        }
    }

    /// Logs an edge.
    /// Panics if the edge was already logged.
    ///
    /// # Arguments
    ///
    /// * `edge` - The edge to log.
    ///
    pub fn log_edge(&self, edge: SL) {
        trace!("log_edge({:?})", edge);
        let mut edge_log = self.edge_log.write().unwrap();
        assert!(
            edge_log.insert(edge),
            "duplicate edge ({:?}) detected",
            edge
        );
    }

    /// Reset the edge logger by clearing the hash-set of edges.
    /// This function is called at the end of each GC iteration.
    ///
    pub fn reset(&self) {
        let mut edge_log = self.edge_log.write().unwrap();
        edge_log.clear();
    }
}

/// Whether we should check duplicate edges. This depends on the actual plan.
pub fn should_check_duplicate_edges<VM: VMBinding>(plan: &dyn Plan<VM = VM>) -> bool {
    // If a plan allows tracing duplicate edges, we should not run this check.
    !plan.constraints().may_trace_duplicate_edges
}
