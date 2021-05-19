use pfm::PerfEvent;
use std::time::SystemTime;
use std::fmt;

#[derive(Copy, Clone, Debug)]
pub(super) struct WorkCounterBase {
    pub(super) total: f64,
    pub(super) min: f64,
    pub(super) max: f64,
}

pub(super) trait WorkCounterClone {
    fn clone_box(&self) -> Box<dyn WorkCounter>;
}

impl<T: 'static + WorkCounter + Clone> WorkCounterClone for T {
    fn clone_box(&self) -> Box<dyn WorkCounter> {
        Box::new(self.clone())
    }
}

pub(super) trait WorkCounter: WorkCounterClone + std::fmt::Debug {
    // TODO: consolidate with crate::util::statistics::counter::Counter;
    fn start(&mut self);
    fn stop(&mut self);
    fn name(&self) -> String;
    fn get_base(&self) -> &WorkCounterBase;
    fn get_base_mut(&mut self) -> &mut WorkCounterBase;
}

impl Clone for Box<dyn WorkCounter> {
    fn clone(&self) -> Box<dyn WorkCounter> {
        self.clone_box()
    }
}

impl Default for WorkCounterBase {
    fn default() -> Self {
        WorkCounterBase {
            total: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
}

impl WorkCounterBase {
    pub(super) fn merge(&self, other: &Self) -> Self {
        let min = self.min.min(other.min);
        let max = self.max.max(other.max);
        let total = self.total + other.total;
        WorkCounterBase { total, min, max }
    }

    pub(super) fn merge_inplace(&mut self, other: &Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
        self.total += other.total;
    }

    pub(super) fn merge_val(&mut self, val: f64) {
        self.min = self.min.min(val);
        self.max = self.max.max(val);
        self.total += val;
    }
}

#[derive(Copy, Clone, Debug)]
pub(super) struct WorkDuration {
    base: WorkCounterBase,
    start_value: Option<SystemTime>,
    running: bool,
}

impl WorkDuration {
    pub(super) fn new() -> Self {
        WorkDuration {
            base: Default::default(),
            start_value: None,
            running: false,
        }
    }
}

impl WorkCounter for WorkDuration {
    fn start(&mut self) {
        self.start_value = Some(SystemTime::now());
        self.running = true;
    }

    fn stop(&mut self) {
        let duration = self.start_value.unwrap().elapsed().unwrap().as_nanos() as f64;
        self.base.merge_val(duration);
    }

    fn name(&self) -> String {
        "time".to_owned()
    }

    fn get_base(&self) -> &WorkCounterBase {
        &self.base
    }

    fn get_base_mut(&mut self) -> &mut WorkCounterBase {
        &mut self.base
    }
}

#[derive(Copy, Clone)]
pub(super) struct WorkPerfEvent {
    base: WorkCounterBase,
    running: bool,
    event_name: &'static str,
    pe: PerfEvent,
}

impl WorkPerfEvent {
    pub(super) fn new(name: &'static str) -> WorkPerfEvent {
        let mut pe = PerfEvent::new(name).expect(&format!("Failed to create perf event {}", name));
        pe.open().expect(&format!("Failed to open perf event {}", name));
        WorkPerfEvent {
            base: Default::default(),
            running: false,
            event_name: name,
            pe,
        }
    }
}

impl fmt::Debug for WorkPerfEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkPerfEvent")
         .field("base", &self.base)
         .field("running", &self.running)
         .field("event_name", &self.event_name)
         .finish()
    }
}

impl WorkCounter for WorkPerfEvent {
    fn start(&mut self) {
        self.running = true;
        self.pe.reset();
        self.pe.enable();
    }
    fn stop(&mut self) {
        self.running = true;
        let perf_event_value = self.pe.read().unwrap();
        self.base.merge_val(perf_event_value.value as f64);
        // assert not multiplexing
        assert_eq!(perf_event_value.time_enabled, perf_event_value.time_running);
        self.pe.disable();
    }
    fn name(&self) -> String {
        self.event_name.to_owned()
    }
    fn get_base(&self) -> &WorkCounterBase {
        &self.base
    }
    fn get_base_mut(&mut self) -> &mut WorkCounterBase {
        &mut self.base
    }
}
