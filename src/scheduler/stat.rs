use std::any::TypeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

#[derive(Default)]
pub struct SchedulerStat {
    work_id_name_map: HashMap<TypeId, &'static str>,
    work_counts: HashMap<TypeId, usize>,
    work_durations: HashMap<TypeId, Vec<WorkDuration>>,
}

trait SimpleCounter {
    // TODO: consolidate with crate::util::statistics::counter::Counter;
    fn start(&mut self);
    fn stop(&mut self);
}

#[derive(Copy, Clone)]
struct WorkDuration {
    total: f64,
    min: f64,
    max: f64,
    start_value: Option<SystemTime>,
    running: bool,
}

impl WorkDuration {
    fn new() -> Self {
        WorkDuration {
            total: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            start_value: None,
            running: false,
        }
    }

    fn process_duration(&mut self, duration: f64) {
        self.min = self.min.min(duration);
        self.max = self.max.max(duration);
        self.total = self.total + duration;
    }

    fn merge_duration(&self, other: &Self) -> Self {
        let min = self.min.min(other.min);
        let max = self.max.max(other.max);
        let total = self.total + other.total;
        WorkDuration {
            total,
            min,
            max,
            start_value: None,
            running: false,
        }
    }

    fn merge_duration_inplace(&mut self, other: &Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
        self.total = self.total + other.total;
    }
}
impl SimpleCounter for WorkDuration {
    fn start(&mut self) {
        self.start_value = Some(SystemTime::now());
        self.running = true;
    }

    fn stop(&mut self) {
        let duration = self.start_value.unwrap().elapsed().unwrap().as_nanos() as f64;
        self.process_duration(duration);
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
        let mut duration_overall = WorkDuration::new();
        for (t, durations) in &self.work_durations {
            let n = self.work_id_name_map[t];
            let fold = durations
                .iter()
                .fold(WorkDuration::new(), |acc, x| acc.merge_duration(x));
            duration_overall.merge_duration_inplace(&fold);
            stat.insert(
                format!("work.{}.time.total", self.work_name(n)),
                format!("{:.2}", fold.total),
            );
            stat.insert(
                format!("work.{}.time.min", self.work_name(n)),
                format!("{:.2}", fold.min),
            );
            stat.insert(
                format!("work.{}.time.max", self.work_name(n)),
                format!("{:.2}", fold.max),
            );
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
        for (id, duration) in &stat.work_durations {
            self.work_durations
                .entry(*id)
                .and_modify(|v| v.push(*duration))
                .or_insert(vec![*duration]);
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
            .work_durations
            .entry(self.type_id)
            .and_modify(|v| v.stop());
    }
}

#[derive(Default)]
pub struct WorkerLocalStat {
    work_id_name_map: HashMap<TypeId, &'static str>,
    work_counts: HashMap<TypeId, usize>,
    work_durations: HashMap<TypeId, WorkDuration>,
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
        self.work_durations
            .entry(work_id)
            .or_insert(WorkDuration::new())
            .start();
        stat
    }
}
