use crate::util::statistics::stats::Stats;
use crate::plan::Phase::{self, *};
use std::sync::{Arc, Mutex};
use crate::util::statistics::{Timer, Counter};

pub struct PhaseTimer {
    ref_type_time: Arc<Mutex<Timer>>,
    scan_time: Arc<Mutex<Timer>>,
    finalize_time: Arc<Mutex<Timer>>,
    prepare_time: Arc<Mutex<Timer>>,
    stacks_time: Arc<Mutex<Timer>>,
    root_time: Arc<Mutex<Timer>>,
    forward_time: Arc<Mutex<Timer>>,
    release_time: Arc<Mutex<Timer>>
}

impl PhaseTimer{
    pub fn new(stats: &Stats) -> Self {
        PhaseTimer {
            ref_type_time: stats.new_timer("refType", false, true),
            scan_time:     stats.new_timer("scan", false, true),
            finalize_time: stats.new_timer("finalize", false, true),
            prepare_time:  stats.new_timer("prepare", false, true),
            stacks_time:   stats.new_timer("stacks", false, true),
            root_time:     stats.new_timer("root", false, true),
            release_time:  stats.new_timer("release", false, true),
            forward_time:  stats.new_timer("forward", false, true),
        }
    }

    fn get_timer<'a, 'b: 'a>(&'a self, p: &'b Phase) -> Option<&'a Mutex<Timer>> {
        match p {
            Prepare => Some(&self.prepare_time),
            StackRoots => Some(&self.stacks_time),
            Roots => Some(&self.root_time),
            Closure => Some(&self.scan_time),
            SoftRefs => Some(&self.ref_type_time),
            WeakRefs => Some(&self.ref_type_time),
            WeakTrackRefs => Some(&self.ref_type_time),
            PhantomRefs => Some(&self.ref_type_time),
            ForwardRefs => Some(&self.ref_type_time),
            Finalizable => Some(&self.finalize_time),
            ForwardFinalizable => Some(&self.finalize_time),
            Forward => Some(&self.forward_time),
            Release => Some(&self.release_time),
            Complex(_, _, Some(ref t)) => Some(t),
            _ => None
        }
    }

    pub fn start_timer(&self, p: &Phase) {
        if let Some(t) = self.get_timer(p) {
            let mut lock = t.lock().unwrap();
            lock.start();
        }
    }

    pub fn stop_timer(&self, p: &Phase) {
        if let Some(t) = self.get_timer(p) {
            let mut lock = t.lock().unwrap();
            lock.stop();
        }
    }
}