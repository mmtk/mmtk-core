use crate::util::analysis::RtAnalysis;
use crate::util::statistics::counter::EventCounter;
use crate::util::statistics::stats::Stats;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/**
 * This file implements an analysis routine that counts the number of objects allocated
 * in each size class. Here, a size class 'sizeX' is defined as 'X bytes or lower'. For
 * example, size64 is the size class with objects <= 64 bytes but >= 32 bytes which is
 * previous size class.
 *
 * We keep track of the size classes using a HashMap with the key being the name of the
 * size class.
 */
#[derive(Default)]
pub struct ObjSize {
    size_classes: Mutex<HashMap<String, Arc<Mutex<EventCounter>>>>,
}

// We require access to both the stats instance in order to create counters for size classes
// on the fly as well as the size of the object allocated
pub struct ObjSizeArgs<'a> {
    stats: &'a Stats,
    size: usize,
}

impl<'a> ObjSizeArgs<'a> {
    pub fn new(stats: &'a Stats, size: usize) -> Self {
        Self { stats, size }
    }
}

// Macro to simplify the creation of a new counter for a particular size class
macro_rules! new_ctr {
    ( $stats:expr, $map:expr, $size:expr ) => {{
        let name = format!("size{}", $size);
        let ctr = $stats.new_event_counter(&name, true, true);
        $map.insert(name, ctr.clone());
        ctr
    }};
}

impl ObjSize {
    pub fn new() -> Self {
        Self {
            size_classes: Mutex::new(HashMap::new()),
        }
    }

    // Fastest way to compute the smallest power of 2 that is larger than n
    // See: https://stackoverflow.com/questions/3272424/compute-fast-log-base-2-ceiling/51351885#51351885
    fn size_class(&self, size: usize) -> usize {
        2_usize.pow(63_u32 - (size - 1).leading_zeros() + 1)
    }
}

impl RtAnalysis<ObjSizeArgs<'_>> for ObjSize {
    fn alloc_hook(&mut self, args: ObjSizeArgs) {
        let stats = args.stats;
        let size = args.size;
        let size_class = format!("size{}", self.size_class(size));
        let mut size_classes = self.size_classes.lock().unwrap();

        let c = size_classes.get_mut(&size_class);
        match c {
            None => {
                // Create (and increment) the counter associated with the size class if it doesn't exist
                let ctr = new_ctr!(stats, size_classes, self.size_class(size));
                ctr.lock().unwrap().inc();
            }
            Some(ctr) => {
                // Increment counter associated with the size class
                ctr.lock().unwrap().inc();
            }
        }
    }
}
