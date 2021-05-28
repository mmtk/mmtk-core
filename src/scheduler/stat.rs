//! Statistics for work packets
use super::work_counter::{WorkCounter, WorkCounterBase, WorkDuration};
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// Merge and print the work-packet level statistics from all worker threads
#[derive(Default)]
pub struct SchedulerStat {
    /// Map work packet type IDs to work packet names
    work_id_name_map: HashMap<TypeId, &'static str>,
    /// Count the number of work packets executed for different types
    work_counts: HashMap<TypeId, usize>,
    /// Collect work counters from work threads.
    /// Two dimensional vectors are used, e.g.
    /// `[[foo_0, ..., foo_n], ..., [bar_0, ..., bar_n]]`.
    /// The first dimension is for different types of work counters,
    /// (`foo` and `bar` in the above example).
    /// The second dimension if for work counters of the same type but from
    /// different threads (`foo_0` and `bar_0` are from the same thread).
    /// The order of insertion is determined by when [`SchedulerStat::merge`] is
    /// called for each [`WorkerLocalStat`].
    /// We assume different threads have the same set of work counters
    /// (in the same order).
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

    /// Used during statistics printing at [`crate::memory_manager::harness_end`]
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
            // Name of the work packet type
            let n = self.work_id_name_map[t];
            // Iterate through different types of work counters
            for v in vs.iter() {
                // Aggregate work counters of the same type but from different
                // worker threads
                let fold = v
                    .iter()
                    .fold(Default::default(), |acc: WorkCounterBase, x| {
                        acc.merge(x.get_base())
                    });
                // Update the overall execution time
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
        // Print out overall execution time
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
    /// Merge work counters from different worker threads
    pub fn merge(&mut self, stat: &WorkerLocalStat) {
        // Merge work packet type ID to work packet name mapping
        for (id, name) in &stat.work_id_name_map {
            self.work_id_name_map.insert(*id, *name);
        }
        // Merge work count for different work packet types
        for (id, count) in &stat.work_counts {
            if self.work_counts.contains_key(id) {
                *self.work_counts.get_mut(id).unwrap() += *count;
            } else {
                self.work_counts.insert(*id, *count);
            }
        }
        // Merge work counter for different work packet types
        for (id, counters) in &stat.work_counters {
            // Initialize the two dimensional vector
            // [
            //    [], // foo counter
            //    [], // bar counter
            // ]
            let vs = self
                .work_counters
                .entry(*id)
                .or_insert_with(|| vec![vec![]; counters.len()]);
            // [
            //    [counters[0] of type foo],
            //    [counters[1] of type bar]
            // ]
            for (v, c) in vs.iter_mut().zip(counters.iter()) {
                v.push(c.clone());
            }
        }
    }
}

/// Describing a single work packet
pub struct WorkStat {
    type_id: TypeId,
    type_name: &'static str,
}

impl WorkStat {
    /// Stop all work counters for the work packet type of the just executed
    /// work packet
    #[inline(always)]
    pub fn end_of_work(&self, worker_stat: &mut WorkerLocalStat) {
        if !worker_stat.is_enabled() {
            return;
        };
        // Insert type ID, name pair
        worker_stat
            .work_id_name_map
            .insert(self.type_id, self.type_name);
        // Increment work count
        *worker_stat.work_counts.entry(self.type_id).or_insert(0) += 1;
        // Stop counters
        worker_stat
            .work_counters
            .entry(self.type_id)
            .and_modify(|v| {
                v.iter_mut().for_each(|c| c.stop());
            });
    }
}

/// Worker thread local counterpart of [`SchedulerStat`]
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
    /// Measure the execution of a work packet by starting all counters for that
    /// type
    #[inline]
    pub fn measure_work(&mut self, work_id: TypeId, work_name: &'static str) -> WorkStat {
        let stat = WorkStat {
            type_id: work_id,
            type_name: work_name,
        };
        if self.is_enabled() {
            self.work_counters
                .entry(work_id)
                .or_insert_with(WorkerLocalStat::counter_set)
                .iter_mut()
                .for_each(|c| c.start());
        }
        stat
    }

    // The set of work counters for all work packet types
    fn counter_set() -> Vec<Box<dyn WorkCounter>> {
        vec![Box::new(WorkDuration::new())]
    }
}
