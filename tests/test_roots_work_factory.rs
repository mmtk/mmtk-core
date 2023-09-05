//! This is for testing the assumption that RootsWorkFactory can work with embedded Box or Arc
//! to hold or share large components in the heap.  Real-world RootsWorkFactory implementations
//! should not be this complicated, and should probably not have shared mutable states, if they
//! have any mutable states at all.

use std::sync::{Arc, Mutex};

use mmtk::{
    util::{Address, ObjectReference},
    vm::RootsWorkFactory,
};

#[derive(Default)]
struct MockScanning {
    roots: Vec<Address>,
}

impl MockScanning {
    fn add_roots(&mut self, roots: &[Address]) {
        self.roots.extend(roots);
    }

    fn mock_scan_roots(&self, mut factory: impl mmtk::vm::RootsWorkFactory<Address>) {
        factory.create_process_edge_roots_work(self.roots.clone());
    }
}

static EDGES: [Address; 3] = [
    unsafe { Address::from_usize(0x8) },
    unsafe { Address::from_usize(0x8) },
    unsafe { Address::from_usize(0x8) },
];

/// A factory with a plain value, a boxed value and a shared data with Arc.
#[derive(Clone)]
struct MockFactory {
    round: i32,
    v: String,
    #[allow(clippy::box_collection)] // for testing `Box` inside a factory
    b: Box<String>,
    a: Arc<Mutex<String>>,
}

impl RootsWorkFactory<Address> for MockFactory {
    fn create_process_edge_roots_work(&mut self, edges: Vec<Address>) {
        assert_eq!(edges, EDGES);
        match self.round {
            1 => {
                assert_eq!(self.v, "y");
                assert_eq!(*self.b, "b");
                assert_eq!(self.a.lock().unwrap().clone(), "a");
            }
            2 => {
                assert_eq!(self.v, "y");
                assert_eq!(*self.b, "b");
                assert_eq!(self.a.lock().unwrap().clone(), "a2");
            }
            3 => {
                assert_eq!(self.v, "y2");
                assert_eq!(*self.b, "b2");
                assert_eq!(self.a.lock().unwrap().clone(), "a2");
            }
            _ => {
                panic!("Unreachable");
            }
        }
    }

    fn create_process_pinning_roots_work(&mut self, _nodes: Vec<ObjectReference>) {
        unimplemented!();
    }

    fn create_process_tpinning_roots_work(&mut self, _nodes: Vec<ObjectReference>) {
        unimplemented!();
    }
}

#[test]
fn test_scan() {
    let factory = MockFactory {
        round: 1,
        v: "y".to_string(),
        b: Box::new("b".to_string()),
        a: Arc::new(Mutex::new("a".to_string())),
    };
    let mut scanning = MockScanning::default();
    scanning.add_roots(&EDGES);
    scanning.mock_scan_roots(factory);
}

#[test]
fn test_clone() {
    let factory1 = MockFactory {
        round: 2,
        v: "y".to_string(),
        b: Box::new("b".to_string()),
        a: Arc::new(Mutex::new("a".to_string())),
    };

    let mut factory2 = factory1.clone();
    factory2.round = 3;
    factory2.v = "y2".to_string();
    *factory2.b = "b2".to_string();
    *factory2.a.lock().unwrap() = "a2".to_string();

    let mut scanning = MockScanning::default();

    scanning.add_roots(&EDGES);
    scanning.mock_scan_roots(factory1);
    scanning.mock_scan_roots(factory2);
}
