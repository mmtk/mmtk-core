use super::mock_test_prelude::*;

use crate::util::heap::gc_trigger::GCTriggerPolicy;
use crate::policy::space::Space;
use crate::plan::Plan;
use crate::util::options::GCTriggerSelector;
use crate::util::conversions;
use crate::MMTK;

use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

const DEFAULT_COLLECT_INTERVAL: usize = 5600 * 1024 * std::mem::size_of::<usize>();
const MAX_COLLECT_INTERVAL: usize = 1250000000;
const GC_ALWAYS_SWEEP_FULL: bool = false;

struct JuliaGCTrigger {
    total_mem: AtomicUsize,
    max_total_memory: AtomicUsize,
    interval: AtomicUsize,
    actual_allocd: AtomicUsize,
    prev_sweep_full: AtomicBool,
    size_hint: usize,

    last_recorded_reserved_pages: AtomicUsize,
}

impl JuliaGCTrigger {
    fn new(total_mem: usize, constrained_mem: usize, size_hint: usize) -> Self {
        // ported from jl_gc_init 64bits
        let mut total_mem = total_mem;
        let mut constrained_mem = constrained_mem;
        if constrained_mem > 0 && constrained_mem < total_mem {
            total_mem = constrained_mem;
        }
        let percent: f64 = if (total_mem as f64) < 123e9 {
            // 60% at 0 gigs and 90% at 128 to not
            // overcommit too much on memory contrained devices
            (total_mem as f64) * 2.34375e-12 + 0.6
        } else {
            0.9f64
        };
        let max_total_memory = if size_hint > 0 && size_hint < (1usize << (std::mem::size_of::<usize>() * 8 - 1)) {
            size_hint as f64
        } else {
            (total_mem as f64) * percent
        };

        Self {
            total_mem: AtomicUsize::new(total_mem),
            max_total_memory: AtomicUsize::new(max_total_memory as usize),
            interval: AtomicUsize::new(DEFAULT_COLLECT_INTERVAL),
            actual_allocd: AtomicUsize::new(0),
            prev_sweep_full: AtomicBool::new(true),
            size_hint,
            last_recorded_reserved_pages: AtomicUsize::new(0),
        }
    }
}

impl GCTriggerPolicy<MockVM> for JuliaGCTrigger {
    fn on_gc_start(&self, mmtk: &'static MMTK<MockVM>) {
        let reserved_pages_in_last_gc = self.last_recorded_reserved_pages.load(Ordering::Relaxed);
        let reserved_pages_now = mmtk.get_plan().get_reserved_pages();
        self.last_recorded_reserved_pages.store(reserved_pages_now, Ordering::Relaxed);
        self.actual_allocd.store(conversions::pages_to_bytes(reserved_pages_now.saturating_sub(reserved_pages_in_last_gc)), Ordering::Relaxed);
        self.prev_sweep_full.store(if let Some(gen) = mmtk.get_plan().generational() {
            gen.last_collection_full_heap()
        } else {
            false
        }, Ordering::Relaxed);
    }
    fn on_gc_end(&self, mmtk: &'static MMTK<MockVM>) {
        let reserved_pages_before_gc = self.last_recorded_reserved_pages.load(Ordering::Relaxed);
        let reserved_pages_now = mmtk.get_plan().get_reserved_pages();
        let freed = conversions::pages_to_bytes(reserved_pages_before_gc.saturating_sub(reserved_pages_now));
        self.last_recorded_reserved_pages.store(reserved_pages_now, Ordering::Relaxed);

        // ported from gc.c -- before sweeping in the original code.
        // ignore large frontier (large frontier means the bytes of pointers reachable from the remset is larger than the default collect interval)
        let gc_auto = !mmtk.state.is_user_triggered_collection();
        let not_freed_enough = gc_auto && (freed as f64) < (self.actual_allocd.load(Ordering::Relaxed) as f64 * 0.7f64);
        let mut sweep_full = false;
        if gc_auto {
            if not_freed_enough {
                self.interval.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |interval| Some(interval * 2));
            }

            // on a big memory machine, increase max_collect_interval to totalmem / nthreads / 2
            // nthreads was computed as 'gc_n_threads - jl_n_gcthreads'. We use mmtk threads.
            let mut maxmem = self.total_mem.load(Ordering::Relaxed) / *mmtk.options.threads / 2;
            if maxmem < MAX_COLLECT_INTERVAL {
                maxmem = MAX_COLLECT_INTERVAL;
            }
            if self.interval.load(Ordering::Relaxed) > maxmem {
                sweep_full = true;
                self.interval.store(maxmem, Ordering::Relaxed);
            }
        }

        if conversions::pages_to_bytes(reserved_pages_now) > self.max_total_memory.load(Ordering::Relaxed) {
            sweep_full = true;
        }
        if GC_ALWAYS_SWEEP_FULL {
            sweep_full = true;
        }
        if let Some(gen) = mmtk.get_plan().generational() {
            if !gen.is_current_gc_nursery() && !self.prev_sweep_full.load(Ordering::Relaxed) {
                sweep_full = true;
                // recollect=1
            }
        }

        if sweep_full {
            if let Some(gen) = mmtk.get_plan().generational() {
                // Force full heap in the next GC
                gen.force_full_heap_collection();
            }
        }

        // ported from gc.c -- after sweeping in the original code
        let live_bytes = conversions::pages_to_bytes(reserved_pages_now);
        if gc_auto {
            // If we aren't freeing enough or are seeing lots and lots of pointers (large_frontier, ignored) let it increase faster
            if not_freed_enough {
                let tot = 2f64 * (live_bytes + self.actual_allocd.load(Ordering::Relaxed)) as f64 / 3f64;
                self.interval.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |interval| if (interval as f64) > tot { Some(tot as usize) } else { None });
            } else {
                // If the current interval is larger than half the live data decrease the interval
                let half = live_bytes / 2;
                self.interval.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |interval| if interval > half { Some(half) } else { None });
            } // self.interval === gc_num.interval

            // But never go below default
            self.interval.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |interval| if interval < DEFAULT_COLLECT_INTERVAL { Some(DEFAULT_COLLECT_INTERVAL) } else { None });
        }

        let max_total_memory = self.max_total_memory.load(Ordering::Relaxed);
        if self.interval.load(Ordering::Relaxed) + live_bytes > max_total_memory {
            if live_bytes < max_total_memory {
                self.interval.store(max_total_memory.saturating_sub(live_bytes), Ordering::Relaxed);
            } else {
                // We can't stay under our goal so let's go back to
                // the minimum interval and hope things get better
                self.interval.store(DEFAULT_COLLECT_INTERVAL, Ordering::Relaxed);
            }
        }
    }

    /// Is a GC required now?
    fn is_gc_required(
        &self,
        space_full: bool,
        space: Option<&dyn Space<MockVM>>,
        plan: &dyn Plan<VM = MockVM>,
    ) -> bool {
        let allocd_so_far = conversions::pages_to_bytes(plan.get_reserved_pages() - self.last_recorded_reserved_pages.load(Ordering::Relaxed));
        if allocd_so_far > self.interval.load(Ordering::Relaxed) {
            return true;
        }

        plan.collection_required(space_full, space)
    }

    /// Is current heap full?
    fn is_heap_full(&self, plan: &dyn Plan<VM = MockVM>) -> bool {
        false
    }

    /// Return the current heap size (in pages)
    fn get_current_heap_size_in_pages(&self) -> usize {
        if self.size_hint > 0 {
            conversions::bytes_to_pages_up(self.size_hint)
        } else {
            usize::MAX
        }
    }

    /// Return the upper bound of heap size
    fn get_max_heap_size_in_pages(&self) -> usize {
        if self.size_hint > 0 {
            conversions::bytes_to_pages_up(self.size_hint)
        } else {
            usize::MAX
        }
    }

    /// Can the heap size grow?
    fn can_heap_size_grow(&self) -> bool {
        true
    }
}

#[test]
pub fn julia_style_gc_trigger() {
    with_mockvm(
        || -> MockVM {
            let total = crate::util::memory::get_system_total_memory() as usize;
            MockVM {
                create_gc_trigger: MockMethod::new_fixed(Box::new(move |_| Some(Box::new(JuliaGCTrigger::new(total, total, 0))))),
                ..MockVM::default()
            }
        },
        || {
            let fixture = MutatorFixture::create_with_builder(|builder| {
                builder.options.gc_trigger.set(GCTriggerSelector::Delegated);
            });
        },
        no_cleanup
    )
}
