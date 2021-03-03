use super::*;
use std::sync::Arc;
use std::sync::Mutex;

pub struct SizeCounter {
    units: Arc<Mutex<EventCounter>>,
    volume: Arc<Mutex<EventCounter>>,
}

/**
 * This file implements a simple counter of events of different sizes
 * (eg object allocations, where total number of objects and total
 * volume of objects would be counted).
 *
 * The counter is trivially composed from two event counters (one for
 * counting the number of events, the other for counting the volume).
 */
impl SizeCounter {
    pub fn new(units: Arc<Mutex<EventCounter>>, volume: Arc<Mutex<EventCounter>>) -> Self {
        SizeCounter { units, volume }
    }

    /**
     * Increment the event counter by provided value
     */
    pub fn inc(&mut self, size: u64) {
        self.units.lock().unwrap().inc();
        self.volume.lock().unwrap().inc_by(size);
    }

    /**
     * Start this counter
     */
    pub fn start(&mut self) {
        self.units.lock().unwrap().start();
        self.volume.lock().unwrap().start();
    }

    /**
     * Stop this counter
     */
    pub fn stop(&mut self) {
        self.units.lock().unwrap().stop();
        self.volume.lock().unwrap().stop();
    }

    /**
     * Print current (mid-phase) units
     */
    pub fn print_current_units(&self) {
        self.units.lock().unwrap().print_current();
    }

    /**
     * Print (mid-phase) volume
     */
    pub fn print_current_volume(&self) {
        self.volume.lock().unwrap().print_current();
    }

    /**
     * Print units
     */
    pub fn print_units(&self) {
        self.units.lock().unwrap().print_total(None);
    }

    /**
     * Print volume
     */
    pub fn print_volume(&self) {
        self.volume.lock().unwrap().print_total(None);
    }
}
