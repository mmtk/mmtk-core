use std::any::TypeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

#[derive(Default)]
pub struct SchedulerStat {
    work_id_name_map: HashMap<TypeId, &'static str>,
    work_counts: HashMap<TypeId, usize>,
    work_counters: HashMap<TypeId, Vec<Vec<Box<dyn WorkCounter>>>>,
}

#[derive(Copy, Clone)]
struct WorkCounterBase {
    total: f64,
    min: f64,
    max: f64,
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
    fn merge(&self, other: &Self) -> Self {
        let min = self.min.min(other.min);
        let max = self.max.max(other.max);
        let total = self.total + other.total;
        WorkCounterBase { total, min, max }
    }

    fn merge_inplace(&mut self, other: &Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
        self.total = self.total + other.total;
    }

    fn merge_val(&mut self, val: f64) {
        self.min = self.min.min(val);
        self.max = self.max.max(val);
        self.total = self.total + val;
    }
}

trait WorkCounter: WorkCounterClone {
    // TODO: consolidate with crate::util::statistics::counter::Counter;
    fn start(&mut self);
    fn stop(&mut self);
    fn name(&self) -> &'static str;
    fn get_base(&self) -> &WorkCounterBase;
    fn get_base_mut(&mut self) -> &mut WorkCounterBase;
}

trait WorkCounterClone {
    fn clone_box(&self) -> Box<dyn WorkCounter>;
}

impl<T: 'static + WorkCounter + Clone> WorkCounterClone for T {
    fn clone_box(&self) -> Box<dyn WorkCounter> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn WorkCounter> {
    fn clone(&self) -> Box<dyn WorkCounter> {
        self.clone_box()
    }
}

#[derive(Copy, Clone)]
struct WorkDuration {
    base: WorkCounterBase,
    start_value: Option<SystemTime>,
    running: bool,
}

impl WorkDuration {
    fn new() -> Self {
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

    fn name(&self) -> &'static str {
        "time"
    }

    fn get_base(&self) -> &WorkCounterBase {
        &self.base
    }

    fn get_base_mut(&mut self) -> &mut WorkCounterBase {
        &mut self.base
    }
}

impl SchedulerStat {
    /// Extract the work-packet name from the full type name.
    /// i.e. simplifies `crate::scheduler::gc_work::SomeWorkPacket<Semispace>` to `SomeWorkPacket`.
    fn work_name(&self, name: &str) -> String {
        let end_index = name.find('<').unwrap_or_else(|| name.len());
        let name = name[..end_index].to_owned();
        match name.rfind(':') {
            Some(start_index) => name[(start_index + 1)..end_index].to_owned(),
            _ => name,
        }
    }

    pub fn harness_stat(&self) -> HashMap<String, String> {
        let mut stat = HashMap::new();
        // Work counts
        let mut total_count = 0;
        for (t, c) in &self.work_counts {
            total_count += c;
            let n = self.work_id_name_map[t];
            stat.insert(
                format!("work.{}.count", self.work_name(n)),
                format!("{}", c),
            );
        }
        stat.insert("total-work.count".to_owned(), format!("{}", total_count));
        // Work execution times
        let mut duration_overall: WorkCounterBase = Default::default();
        for (t, vs) in &self.work_counters {
            let n = self.work_id_name_map[t];
            for v in vs.iter() {
                let fold = v
                    .iter()
                    .fold(Default::default(), |acc: WorkCounterBase, x| {
                        acc.merge(x.get_base())
                    });
                duration_overall.merge_inplace(&fold);
                let name = v.first().unwrap().name();
                stat.insert(
                    format!("work.{}.{}.total", self.work_name(n), name),
                    format!("{:.2}", fold.total),
                );
                stat.insert(
                    format!("work.{}.{}.min", self.work_name(n), name),
                    format!("{:.2}", fold.min),
                );
                stat.insert(
                    format!("work.{}.{}.max", self.work_name(n), name),
                    format!("{:.2}", fold.max),
                );
            }
        }

        stat.insert(
            "total-work.time.total".to_owned(),
            format!("{:.2}", duration_overall.total),
        );
        stat.insert(
            "total-work.time.min".to_owned(),
            format!("{:.2}", duration_overall.min),
        );
        stat.insert(
            "total-work.time.max".to_owned(),
            format!("{:.2}", duration_overall.max),
        );

        stat
    }

    pub fn merge(&mut self, stat: &WorkerLocalStat) {
        for (id, name) in &stat.work_id_name_map {
            self.work_id_name_map.insert(*id, *name);
        }
        for (id, count) in &stat.work_counts {
            if self.work_counts.contains_key(id) {
                *self.work_counts.get_mut(id).unwrap() += *count;
            } else {
                self.work_counts.insert(*id, *count);
            }
        }
        for (id, counters) in &stat.work_counters {
            let vs = self
                .work_counters
                .entry(*id)
                .or_insert(vec![vec![]; counters.len()]);
            for (v, c) in vs.iter_mut().zip(counters.iter()) {
                v.push(c.clone());
            }
        }
    }
}

pub struct WorkStat {
    type_id: TypeId,
    type_name: &'static str,
}

impl WorkStat {
    #[inline(always)]
    pub fn end_of_work(&self, worker_stat: &mut WorkerLocalStat) {
        if !worker_stat.is_enabled() {
            return;
        };
        worker_stat
            .work_id_name_map
            .insert(self.type_id, self.type_name);
        *worker_stat.work_counts.entry(self.type_id).or_insert(0) += 1;
        worker_stat
            .work_counters
            .entry(self.type_id)
            .and_modify(|v| {
                v.iter_mut().for_each(|c| c.stop());
            });
    }
}

#[derive(Default)]
pub struct WorkerLocalStat {
    work_id_name_map: HashMap<TypeId, &'static str>,
    work_counts: HashMap<TypeId, usize>,
    work_counters: HashMap<TypeId, Vec<Box<dyn WorkCounter>>>,
    enabled: AtomicBool,
}

impl WorkerLocalStat {
    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    #[inline]
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }
    #[inline]
    pub fn measure_work(&mut self, work_id: TypeId, work_name: &'static str) -> WorkStat {
        let stat = WorkStat {
            type_id: work_id,
            type_name: work_name,
        };
        self.work_counters
            .entry(work_id)
            .or_insert(WorkerLocalStat::counter_set())
            .iter_mut()
            .for_each(|c| c.start());
        stat
    }

    fn counter_set() -> Vec<Box<dyn WorkCounter>> {
        vec![Box::new(WorkDuration::new())]
    }
}
