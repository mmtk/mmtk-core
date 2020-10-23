use std::any::TypeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

#[derive(Default)]
pub struct SchedulerStat {
    work_id_name_map: HashMap<TypeId, &'static str>,
    work_counts: HashMap<TypeId, usize>,
    work_durations: HashMap<TypeId, Vec<Duration>>,
}

impl SchedulerStat {
    fn work_name(&self, name: &str) -> String {
        let end_index = name.find('<').unwrap_or_else(|| name.len());
        let name = name[..end_index].to_owned();
        match name.rfind(':') {
            Some(start_index) => name[(start_index + 1)..end_index].to_owned(),
            _ => name,
        }
    }

    fn geomean(&self, values: &[f64]) -> f64 {
        // Geomean(xs, N=xs.len()) = (PI(xs))^(1/N) = e^{log{PI(xs)^(1/N)}} = e^{ (1/N) * sum_{x \in xs}{ log(x) } }
        let logs = values.iter().map(|v| v.ln());
        let sum_logs = logs.sum::<f64>();
        (sum_logs / values.len() as f64).exp()
    }
    fn min(&self, values: &[f64]) -> f64 {
        let mut min = values[0];
        for v in values {
            if *v < min {
                min = *v
            }
        }
        min
    }
    fn max(&self, values: &[f64]) -> f64 {
        let mut max = values[0];
        for v in values {
            if *v > max {
                max = *v
            }
        }
        max
    }

    pub fn harness_stat(&self) -> HashMap<String, String> {
        let mut stat = HashMap::new();
        // Work counts
        let mut total_count = 0;
        for (t, c) in &self.work_counts {
            total_count += c;
            let n = self.work_id_name_map[t];
            stat.insert(
                format!("works.{}.count", self.work_name(n)),
                format!("{}", c),
            );
        }
        stat.insert("total-works.count".to_owned(), format!("{}", total_count));
        // Work execution times
        let mut total_durations = vec![];
        for (t, durations) in &self.work_durations {
            for d in durations {
                total_durations.push(*d);
            }
            let n = self.work_id_name_map[t];
            let geomean = self.geomean(
                &durations
                    .iter()
                    .map(|d| d.as_nanos() as f64)
                    .collect::<Vec<_>>(),
            );
            stat.insert(
                format!("works.{}.time.geomean", self.work_name(n)),
                format!("{:.2}", geomean),
            );
        }
        let durations = total_durations
            .iter()
            .map(|d| d.as_nanos() as f64)
            .collect::<Vec<_>>();
        if !durations.is_empty() {
            stat.insert(
                "total-works.time.geomean".to_owned(),
                format!("{:.2}", self.geomean(&durations)),
            );
            stat.insert(
                "total-works.time.min".to_owned(),
                format!("{:.2}", self.min(&durations)),
            );
            stat.insert(
                "total-works.time.max".to_owned(),
                format!("{:.2}", self.max(&durations)),
            );
        }

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
        for (id, durations) in &stat.work_durations {
            if self.work_durations.contains_key(id) {
                let work_durations = self.work_durations.get_mut(id).unwrap();
                for d in durations {
                    work_durations.push(*d);
                }
            } else {
                self.work_durations.insert(*id, durations.clone());
            }
        }
    }
}

pub struct WorkStat {
    type_id: TypeId,
    type_name: &'static str,
    start_time: SystemTime,
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
        let duration = self.start_time.elapsed().unwrap();
        worker_stat
            .work_durations
            .entry(self.type_id)
            .or_insert_with(Vec::new)
            .push(duration);
    }
}

#[derive(Default)]
pub struct WorkerLocalStat {
    work_id_name_map: HashMap<TypeId, &'static str>,
    work_counts: HashMap<TypeId, usize>,
    work_durations: HashMap<TypeId, Vec<Duration>>,
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
        WorkStat {
            type_id: work_id,
            type_name: work_name,
            start_time: SystemTime::now(),
        }
    }
}
