use crate::util::analysis::RtAnalysis;
use crate::util::statistics::counter::EventCounter;
use crate::util::statistics::stats::Stats;
use crate::vm::VMBinding;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/**
 * This file implements an analysis routine that counts the number of objects allocated
 * in each size class. Here, a size class 'sizeX' is defined as 'X bytes or lower'. For
 * example, size64 is the size class with objects <= 64 bytes but > 32 bytes which is
 * the previous size class.
 *
 * We keep track of the size classes using a HashMap with the key being the name of the
 * size class.
 */
pub struct PerSizeClassObjectCounter {
    running: bool,
    size_classes: Mutex<HashMap<String, Arc<Mutex<EventCounter>>>>,
    stats: Arc<Stats>,
}

// Macro to simplify the creation of a new counter for a particular size class.
// This is a macro as opposed to a function as otherwise we would have to unlock
// and relock the size_classes map
macro_rules! new_ctr {
    ( $stats:expr, $map:expr, $size_class:expr ) => {{
        let ctr = $stats.new_event_counter(&$size_class, true, true);
        $map.insert($size_class.to_string(), ctr.clone());
        ctr
    }};
}

impl PerSizeClassObjectCounter {
    pub fn new(running: bool, stats: Arc<Stats>) -> Self {
        Self {
            running,
            size_classes: Mutex::new(HashMap::new()),
            stats,
        }
    }

    // Fastest way to compute the smallest power of 2 that is larger than n
    // See: https://stackoverflow.com/questions/3272424/compute-fast-log-base-2-ceiling/51351885#51351885
    fn size_class(&self, size: usize) -> usize {
        2_usize.pow(63_u32 - (size - 1).leading_zeros() + 1)
    }
}

impl<VM: VMBinding> RtAnalysis<VM> for PerSizeClassObjectCounter {
    fn alloc_hook(&mut self, size: usize, _align: usize, _offset: usize) {
        if !self.running {
            return;
        }

        let size_class = format!("size{}", self.size_class(size));
        let mut size_classes = self.size_classes.lock().unwrap();
        let c = size_classes.get_mut(&size_class);
        match c {
            None => {
                // Create (and increment) the counter associated with the size class if it doesn't exist
                let ctr = new_ctr!(self.stats, size_classes, size_class);
                ctr.lock().unwrap().inc();
            }
            Some(ctr) => {
                // Increment counter associated with the size class
                ctr.lock().unwrap().inc();
            }
        }
    }

    fn set_running(&mut self, running: bool) {
        self.running = running;
    }
}
