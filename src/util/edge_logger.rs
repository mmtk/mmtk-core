use super::Address;
use std::collections::HashSet;
use std::sync::RwLock;

lazy_static! {
    static ref EDGE_LOG: RwLock<HashSet<Address>> = RwLock::new(HashSet::new());
}

pub fn log_edge(edge: Address) {
    // println!("log_edge({})", edge);
    let mut edge_log = EDGE_LOG.write().unwrap();
    assert!(edge_log.insert(edge), "duplicate edge ({}) detected", edge);
}

pub fn is_logged_edge(edge: Address) -> bool {
    let edge_log = EDGE_LOG.read().unwrap();
    edge_log.contains(&edge)
}

pub fn reset() {
    let mut edge_log = EDGE_LOG.write().unwrap();
    edge_log.clear();
    // println!("edge_logger::reset()");
}
