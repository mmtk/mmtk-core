use util::statistics::stats::new_counter;
use util::statistics::counter::{LongCounter, MonotoneNanoTime};
use plan::Phase::{self, *};
use util::statistics::stats::COUNTER;

pub struct PhaseTimer {
    ref_type_time: usize,
    scan_time: usize,
    finalize_time: usize,
    prepare_time: usize,
    stacks_time: usize,
    root_time: usize,
    forward_time: usize,
    release_time: usize
}

impl PhaseTimer{
    pub fn new() -> Self {
        PhaseTimer {
            ref_type_time: new_counter(LongCounter::<MonotoneNanoTime>::new("refType".to_string(), false, true)),
            scan_time: new_counter(LongCounter::<MonotoneNanoTime>::new("scan".to_string(), false, true)),
            finalize_time: new_counter(LongCounter::<MonotoneNanoTime>::new("finalize".to_string(), false, true)),
            prepare_time: new_counter(LongCounter::<MonotoneNanoTime>::new("prepare".to_string(), false, true)),
            stacks_time: new_counter(LongCounter::<MonotoneNanoTime>::new("stacks".to_string(), false, true)),
            root_time: new_counter(LongCounter::<MonotoneNanoTime>::new("root".to_string(), false, true)),
            forward_time: new_counter(LongCounter::<MonotoneNanoTime>::new("forward".to_string(), false, true)),
            release_time: new_counter(LongCounter::<MonotoneNanoTime>::new("release".to_string(), false, true))
        }
    }

    fn get_timer_id(&self, p: &Phase) -> Option<usize> {
        match p {
            Prepare => Some(self.prepare_time),
            StackRoots => Some(self.stacks_time),
            Roots => Some(self.root_time),
            Closure => Some(self.scan_time),
            SoftRefs => Some(self.ref_type_time),
            WeakRefs => Some(self.ref_type_time),
            WeakTrackRefs => Some(self.ref_type_time),
            PhantomRefs => Some(self.ref_type_time),
            ForwardRefs => Some(self.ref_type_time),
            Finalizable => Some(self.finalize_time),
            ForwardFinalizable => Some(self.finalize_time),
            Forward => Some(self.forward_time),
            Release => Some(self.release_time),
            Complex(_, _, id) => *id,
            _ => None
        }
    }

    pub fn start_timer(&self, p: &Phase) {
        if let Some(id) = self.get_timer_id(p) {
            let mut counter = COUNTER.lock().unwrap();
            counter[id].start();
        }
    }

    pub fn start_timer_id(&self, id: usize) {
        let mut counter = COUNTER.lock().unwrap();
        counter[id].start();
    }

    pub fn stop_timer(&self, p: &Phase) {
        if let Some(id) = self.get_timer_id(p) {
            let mut counter = COUNTER.lock().unwrap();
            counter[id].stop();
        }
    }

    pub fn stop_timer_id(&self, id: usize) {
        let mut counter = COUNTER.lock().unwrap();
        counter[id].stop();
    }
}