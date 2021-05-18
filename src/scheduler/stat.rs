use super::work_counter::{WorkCounter, WorkCounterBase, WorkDuration, WorkPerfEvent};
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct SchedulerStat {
    work_id_name_map: HashMap<TypeId, &'static str>,
    work_counts: HashMap<TypeId, usize>,
    work_counters: HashMap<TypeId, Vec<Vec<Box<dyn WorkCounter>>>>,
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
                .or_insert_with(|| vec![vec![]; counters.len()]);
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
            .or_insert_with(WorkerLocalStat::counter_set)
            .iter_mut()
            .for_each(|c| c.start());
        stat
    }

    fn counter_set() -> Vec<Box<dyn WorkCounter>> {
        let events = "PERF_COUNT_HW_CPU_CYCLES,PERF_COUNT_HW_INSTRUCTIONS,PERF_COUNT_HW_CACHE_REFERENCES,PERF_COUNT_HW_CACHE_MISSES,PERF_COUNT_HW_CACHE_L1D:MISS,PERF_COUNT_HW_CACHE_L1I:MISS";
        let mut counters: Vec<Box<dyn WorkCounter>> = vec![Box::new(WorkDuration::new())];
        for event in events.split(",") {
            counters.push(Box::new(WorkPerfEvent::new(event)))
        }
        counters
    }
}
