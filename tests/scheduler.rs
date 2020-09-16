/* An implementation of parallel quicksort */

#![feature(is_sorted)]

use mmtk::util::OpaquePointer;
use mmtk::scheduler::*;
use lazy_static::lazy_static;
use std::sync::Arc;
use rand::{thread_rng, Rng};



/// A work-packet to (quick)sort a slice of array
struct Sort(&'static mut [usize]);

impl Work<()> for Sort {
    fn do_work(&mut self, worker: &'static mut Worker<()>, _context: &'static ()) {
        if self.0.len() <= 1 { return /* Do nothing */ }
        worker.scheduler().unconstrained_works.add(Partition(unsafe { &mut *(self.0 as *mut _) }));
    }
}

/// A work-packet to do array partition
///
/// Recursively generates `Sort` works for partitioned sub-arrays.
struct Partition(&'static mut [usize]);

impl Work<()> for Partition {
    fn do_work(&mut self, worker: &'static mut Worker<()>, _context: &'static ()) {
        assert!(self.0.len() >= 2);

        // 1. Partition

        let pivot: usize = self.0[0];
        let le = self.0.iter().skip(1).filter(|v| **v <= pivot).map(|v| *v).collect::<Vec<_>>();
        let gt = self.0.iter().skip(1).filter(|v| **v > pivot).map(|v| *v).collect::<Vec<_>>();

        let pivot_index = le.len();
        for (i, v) in le.iter().enumerate() {
            self.0[i] = *v;
        }
        self.0[pivot_index] = pivot;
        for (i, v) in gt.iter().enumerate() {
            self.0[pivot_index + i + 1] = *v;
        }

        // 2. Create two `Sort` work packets

        let left: &'static mut [usize] = unsafe { &mut *(&mut self.0[.. pivot_index] as *mut _) };
        let right: &'static mut [usize] = unsafe { &mut *(&mut self.0[pivot_index + 1 ..] as *mut _) };

        worker.scheduler().unconstrained_works.add(Sort(left));
        worker.scheduler().unconstrained_works.add(Sort(right));
    }
}



lazy_static! {
    static ref SCHEDULER: Arc<Scheduler<()>> = Scheduler::new();
}

const NUM_WORKERS: usize = 16;

fn random_array(size: usize) -> Box<[usize]> {
    let mut rng = thread_rng();
    (0..size).map(|_| rng.gen()).collect()
}

#[test]
fn quicksort() {
    let data: &'static mut [usize] = Box::leak(random_array(1000));

    println!("Original: {:?}", data);

    SCHEDULER.initialize(NUM_WORKERS, &(), OpaquePointer::UNINITIALIZED);
    SCHEDULER.unconstrained_works.add(Sort(unsafe { &mut *(data as *mut _) }));
    SCHEDULER.wait_for_completion();

    println!("Sorted: {:?}", data);

    assert!(data.is_sorted());

    let _data = unsafe { Box::from_raw(data) };
}