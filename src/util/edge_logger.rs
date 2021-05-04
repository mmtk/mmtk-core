//! This is a simple module to log edges and check for duplicate edges.
//!
//! It uses a hash-set to keep track of edge, and is so very expensive.
//! We currently only use this as part of the `extreme_assertions` feature.
//!

use super::Address;
use std::collections::HashSet;
use std::sync::RwLock;

lazy_static! {
    // A private hash-set to keep track of edges.
    static ref EDGE_LOG: RwLock<HashSet<Address>> = RwLock::new(HashSet::new());
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

/// Checks whether an edge is already logged.
/// Returns `true` if the edge is logged and `false` otherwise.
///
/// # Arguments
///
/// * `edge` - The edge to check.
///
pub fn is_logged_edge(edge: Address) -> bool {
    let edge_log = EDGE_LOG.read().unwrap();
    edge_log.contains(&edge)
}

/// Reset the edge logger by clearing the hash-set of edges.
/// This function is called at the end of each GC iteration.
///
pub fn reset() {
    let mut edge_log = EDGE_LOG.write().unwrap();
    edge_log.clear();
}
