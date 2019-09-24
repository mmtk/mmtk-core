use policy::region::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use super::g1::{GCKind, PLAN};

const PAUSE_TIME_GOAL: usize = 15; // ms
const NURSERY_PAUSE_TIME_GOAL: usize = 10; // ms
const INITIAL_NURSERY_RATIO: f32 = 0.1;
const MIN_NURSERY_RATIO: f32 = 0.05;
const MAX_NURSERY_RATIO: f32 = 0.40;

/**
 * V(cs) = V_{fixed} + U*d + \sum_{r \in cs}{( S*rsSize(r) + C*liveBytes(r) )}
 * 
 * - V(cs) is the cost of collecting collection set cs;
 * - V_{fixed} represents fixed costs, common to all pauses;
 * - U is the average cost of scanning a card
 * - d is the number of dirty cards that must be scanned to bring remembered sets up-to-date;
 * - S is the of scanning a card from a remembered set for pointers into the collection set
 * - rsSize(r) is the number of card entries in r’s remembered set;
 * - C is the cost per byte of evacuating (and scanning) a live object,
 * - liveBytes(r) is an estimate of the number of live bytes in region r.
*/

// We use milliseconds as our time unit for the following calculation

pub struct PauseTimePredictor {
    v_fixed: usize,
    u: f32,
    s: f32,
    c: f32,
    num_workers: usize,
    pub timer: PauseTimePredictionTimer,
    nursery_regions: usize,
    pub nursery_time_per_region: f32,
}

impl PauseTimePredictor {
    pub fn new() -> Self {
        Self {
            v_fixed: 0,
            u: 0f32,
            s: 0f32,
            c: 0f32,
            num_workers: 0,
            nursery_regions: 0,
            nursery_time_per_region: 0.0,
            // nursery_ratio: INITIAL_NURSERY_RATIO,
            timer: PauseTimePredictionTimer::new(),
        }
    }

    pub fn pause_start(&mut self, num_workers: usize, nursery_regions: usize) {
        if !super::ENABLE_PAUSE_TIME_PREDICTOR {
            return;
        }
        self.num_workers = num_workers;
        self.nursery_regions = nursery_regions;
        self.timer.pause_start(num_workers);
    }

    pub fn pause_end(&mut self, gc_kind: GCKind) {
        if !super::ENABLE_PAUSE_TIME_PREDICTOR {
            return;
        }
        if gc_kind == GCKind::Full { return }
        let pause_time = self.timer.pause_end(gc_kind == GCKind::Full);
        self.update_parameters(pause_time, gc_kind);
    }

    fn update_parameters(&mut self, pause_time: usize, gc_kind: GCKind) {
        // println!("[Pause {:?} {} millis]", gc_kind, pause_time);
        if gc_kind != GCKind::Full {
            macro_rules! mix {
                ($a: expr, $b: expr) => {{
                    let (a, b) = ($a, $b);
                    if a > b { a } else { b }
                }};
            }
            self.v_fixed = (self.v_fixed + self.timer.v_fixed()) / 2;
            self.u = mix!(self.u, self.timer.u());
            self.s = mix!(self.s, self.timer.s());
            self.c = mix!(self.c, self.timer.c());
        }

        if gc_kind == GCKind::Young {
            let fixed_time = self.timer.v_fixed();
            // assert!(fixed_time <= pause_time);
            // let free_time = if NURSERY_PAUSE_TIME_GOAL >= fixed_time { NURSERY_PAUSE_TIME_GOAL - fixed_time } else { 0 };
            // let nursery = {
            //     if self.nursery_regions == 0 || fixed_time == pause_time {
            //         0.0
            //     } else {
            //         let time_per_region = (pause_time - fixed_time) as f32 / self.nursery_regions as f32;
            //         free_time as f32 / time_per_region
            //     }
            // };
            let nursery_time_per_region = {
                if self.nursery_regions == 0 || fixed_time == pause_time {
                    0.0
                } else {
                    let time_per_region = (pause_time - fixed_time) as f32 / self.nursery_regions as f32;
                    time_per_region
                }
            };
            if self.nursery_time_per_region == 0.0 {
                self.nursery_time_per_region = nursery_time_per_region;
            } else if nursery_time_per_region != 0.0 {
                self.nursery_time_per_region = (self.nursery_time_per_region + nursery_time_per_region) / 2.0;
            }
            // let total = (PLAN.region_space.heap_size >> LOG_BYTES_IN_REGION) as f32;
            // let nursery_ratio = nursery / total;
            // if nursery_ratio != 0.0 {
            //     self.nursery_ratio = (self.nursery_ratio + nursery_ratio) / 2.0;
            // }
            // if self.nursery_ratio < MIN_NURSERY_RATIO { self.nursery_ratio = MIN_NURSERY_RATIO }
            // if self.nursery_ratio > MAX_NURSERY_RATIO { self.nursery_ratio = MAX_NURSERY_RATIO }
            // println!("[Nursery ratio = {}]", self.nursery_ratio);
        }

    }

    pub fn get_accumulative_predictor(&self, d: usize) -> impl AccumulativePauseTimePredictor {
        // println!("v_fixed = {}", self.v_fixed);
        // println!("U = {} d = {}", self.u, d);
        // println!("U * d = {}", (self.u * d as f32 / self.num_workers as f32));
        let v = self.v_fixed + (self.u * d as f32 / self.num_workers as f32) as usize;
        let (s, c) = (self.s, self.c);
        G1AccumulativePredictor { num_workers: self.num_workers, total_s_c: 0.0, v, s, c }
    }

    
    pub fn within_nursery_budget(&self, d: usize) -> bool {
        if !super::ENABLE_PAUSE_TIME_PREDICTOR {
            return PLAN.region_space.nursery_ratio() > INITIAL_NURSERY_RATIO;
        }
        if self.nursery_time_per_region == 0.0 {
            return PLAN.region_space.nursery_ratio() > INITIAL_NURSERY_RATIO;
        }
        let d = cardtable::num_dirty_cards() as f32;
        let v = self.v_fixed as f32 + self.u as f32 * d / self.num_workers as f32;
        let budget = super::predictor::NURSERY_PAUSE_TIME_GOAL as f32;
        if budget <= v {
            return PLAN.region_space.nursery_ratio() > MIN_NURSERY_RATIO;
        }
        let free = budget - v;
        let nursery_regions = free / self.nursery_time_per_region;
        let r = PLAN.region_space.nursery_regions() >= nursery_regions as usize;
        if r {
            // println!("[Nursery Ratio {}]", PLAN.region_space.nursery_ratio());
        }
        if !r {
            return PLAN.region_space.nursery_ratio() > MAX_NURSERY_RATIO;
        }
        r
    }
}

pub struct G1AccumulativePredictor {
    num_workers: usize,
    v: usize,
    total_s_c: f32,
    s: f32,
    c: f32,
}

impl AccumulativePauseTimePredictor for G1AccumulativePredictor {
    fn record(&mut self, r: RegionRef) {
        // println!("{:?} s={} c={} rs={} live={}", r,  self.s, self.c, r.rs_size(), r.live_size.load(Ordering::SeqCst));
        // println!("Accumulate {}", (self.s * r.rs_size() as f32) as f32 + (self.c * r.live_size.load(Ordering::SeqCst) as f32) as f32);
        self.total_s_c += (self.s * r.rs_size() as f32) as f32 + (self.c * r.live_size() as f32) as f32;
    }
    fn predict(&self) -> usize {
        let v = self.v + (self.total_s_c / self.num_workers as f32) as usize;
        v
    }
    fn predict_f32(&self) -> f32 {
        self.v as f32 + (self.total_s_c / self.num_workers as f32)
    }
    fn within_budget(&self) -> bool {
        if !super::ENABLE_PAUSE_TIME_PREDICTOR {
            return true;
        }
        if self.v >= PAUSE_TIME_GOAL {
            return true;
        }
        self.predict() < PAUSE_TIME_GOAL
    }
}
