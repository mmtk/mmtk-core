//! This is a simple module to log edges and check for duplicate edges.
//!
//! It uses a hash-set to keep track of edge, and is so very expensive.
//! We currently only use this as part of the `extreme_assertions` feature.
//!

use crate::plan::Plan;
use crate::util::Address;
use crate::vm::VMBinding;
use std::collections::HashSet;
use std::sync::RwLock;

lazy_static! {
    // A private hash-set to keep track of edges.
    static ref EDGE_LOG: RwLock<HashSet<Address>> = RwLock::new(HashSet::new());
}

/// Whether we should check duplicate edges. This depends on the actual plan.
pub fn should_check_duplicate_edges<VM: VMBinding>(plan: &dyn Plan<VM = VM>) -> bool {
    // If a plan allows tracing duplicate edges, we should not run this check.
    !plan.constraints().may_trace_duplicate_edges
}

/// Logs an edge.
/// Panics if the edge was already logged.
///
/// # Arguments
///
/// * `edge` - The edge to log.
///
pub fn log_edge(edge: Address) {
    trace!("log_edge({})", edge);
    let mut edge_log = EDGE_LOG.write().unwrap();
    assert!(edge_log.insert(edge), "duplicate edge ({}) detected", edge);
}

/// Reset the edge logger by clearing the hash-set of edges.
/// This function is called at the end of each GC iteration.
///
pub fn reset() {
    let mut edge_log = EDGE_LOG.write().unwrap();
    edge_log.clear();
}
