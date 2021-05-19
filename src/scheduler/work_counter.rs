use std::time::SystemTime;

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
