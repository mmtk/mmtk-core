//! Statistics for work packets
use super::work_counter::{WorkCounter, WorkCounterBase, WorkDuration};
#[cfg(feature = "perf_counter")]
use crate::scheduler::work_counter::WorkPerfEvent;
use crate::vm::VMBinding;
use crate::MMTK;
use std::any::TypeId;
use std::collections::HashMap;
use std::marker::PhantomData;
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
        let end_index = name.find('<').unwrap_or(name.len());
        let name = name[..end_index].to_owned();
        match name.rfind(':') {
            Some(start_index) => name[(start_index + 1)..end_index].to_owned(),
            _ => name,
        }
    }

    /// Used during statistics printing at [`crate::memory_manager::harness_end`]
    pub fn harness_stat(&self) -> HashMap<String, String> {
        let mut stat = HashMap::new();
        let mut counts = HashMap::<String, usize>::new();
        let mut times = HashMap::<String, f64>::new();
        // Work counts
        let mut total_count = 0;
        for (t, c) in &self.work_counts {
            total_count += c;
            let n = self.work_id_name_map[t];
            // We can have the same work names for different TypeIDs since work names strip
            // type parameters away, while the same work packet with different type parameters
            // are given different TypeIDs. Hence, we check if the key exists and update instead of
            // overwrite it
            let pkt = format!("work.{}.count", self.work_name(n));
            let val = counts.entry(pkt).or_default();
            *val += c;
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
                let pkt_total = format!("work.{}.{}.total", self.work_name(n), name);
                let pkt_min = format!("work.{}.{}.min", self.work_name(n), name);
                let pkt_max = format!("work.{}.{}.max", self.work_name(n), name);

                // We can have the same work names for different TypeIDs since work names strip
                // type parameters away, while the same work packet with different type parameters
                // are given different TypeIDs. Hence, we check if the key exists and update
                // instead of overwrite it
                let val = times.entry(pkt_total).or_default();
                *val += fold.total;
                let val = times.entry(pkt_min).or_default();
                *val = f64::min(*val, fold.min);
                let val = times.entry(pkt_max).or_default();
                *val = f64::max(*val, fold.max);
            }
        }
        // Convert to ms and print out overall execution time
        stat.insert(
            "total-work.time.total".to_owned(),
            format!("{:.3}", duration_overall.total / 1e6),
        );
        stat.insert(
            "total-work.time.min".to_owned(),
            format!("{:.3}", duration_overall.min / 1e6),
        );
        stat.insert(
            "total-work.time.max".to_owned(),
            format!("{:.3}", duration_overall.max / 1e6),
        );

        for (pkt, count) in counts {
            stat.insert(pkt, format!("{}", count));
        }

        for (pkt, time) in times {
            stat.insert(pkt, format!("{:.3}", time / 1e6));
        }

        stat
    }
    /// Merge work counters from different worker threads
    pub fn merge<C>(&mut self, stat: &WorkerLocalStat<C>) {
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
    pub fn end_of_work<VM: VMBinding>(&self, worker_stat: &mut WorkerLocalStat<VM>) {
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
pub struct WorkerLocalStat<C> {
    work_id_name_map: HashMap<TypeId, &'static str>,
    work_counts: HashMap<TypeId, usize>,
    work_counters: HashMap<TypeId, Vec<Box<dyn WorkCounter>>>,
    enabled: AtomicBool,
    _phantom: PhantomData<C>,
}

unsafe impl<C> Send for WorkerLocalStat<C> {}

impl<C> Default for WorkerLocalStat<C> {
    fn default() -> Self {
        WorkerLocalStat {
            work_id_name_map: Default::default(),
            work_counts: Default::default(),
            work_counters: Default::default(),
            enabled: AtomicBool::new(false),
            _phantom: Default::default(),
        }
    }
}

impl<VM: VMBinding> WorkerLocalStat<VM> {
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }
    /// Measure the execution of a work packet by starting all counters for that
    /// type
    pub fn measure_work(
        &mut self,
        work_id: TypeId,
        work_name: &'static str,
        mmtk: &'static MMTK<VM>,
    ) -> WorkStat {
        let stat = WorkStat {
            type_id: work_id,
            type_name: work_name,
        };
        if self.is_enabled() {
            self.work_counters
                .entry(work_id)
                .or_insert_with(|| Self::counter_set(mmtk))
                .iter_mut()
                .for_each(|c| c.start());
        }
        stat
    }

    #[allow(unused_variables, unused_mut)]
    fn counter_set(mmtk: &'static MMTK<VM>) -> Vec<Box<dyn WorkCounter>> {
        let mut counters: Vec<Box<dyn WorkCounter>> = vec![Box::new(WorkDuration::new())];
        #[cfg(feature = "perf_counter")]
        for e in &mmtk.options.work_perf_events.events {
            counters.push(Box::new(WorkPerfEvent::new(
                &e.0,
                e.1,
                e.2,
                *mmtk.options.perf_exclude_kernel,
            )));
        }
        counters
    }
}
