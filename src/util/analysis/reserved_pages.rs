use crate::plan::marksweep::MarkSweep;
use crate::util::analysis::RtAnalysis;
use crate::util::statistics::counter::SingleCounter;
use crate::vm::{ActivePlan, VMBinding};
use std::convert::TryInto;
use std::sync::{atomic::Ordering, Arc, Mutex};

pub struct ReservedPagesCounter {
    running: bool,
    reserved_pages_max: Mutex<usize>,
    reserved_pages_max_ctr: Arc<Mutex<SingleCounter>>,
    reserved_pages_all_sum: Mutex<usize>,
    reserved_pages_total: Mutex<usize>,
    reserved_pages_avg_ctr: Arc<Mutex<SingleCounter>>,
}

impl ReservedPagesCounter {
    pub fn new(
        running: bool,
        reserved_pages_max_ctr: Arc<Mutex<SingleCounter>>,
        reserved_pages_avg_ctr: Arc<Mutex<SingleCounter>>,
    ) -> Self {
        Self {
            running,
            reserved_pages_max: Mutex::new(0),
            reserved_pages_max_ctr,
            reserved_pages_all_sum: Mutex::new(0),
            reserved_pages_total: Mutex::new(0),
            reserved_pages_avg_ctr,
        }
    }
}

impl<VM: VMBinding> RtAnalysis<VM> for ReservedPagesCounter {
    fn alloc_hook(&mut self, _size: usize, _align: usize, _offset: isize) {
        if !self.running {
            return;
        }

        let plan = VM::VMActivePlan::global()
            .downcast_ref::<MarkSweep<VM>>()
            .unwrap();
        let rss = plan.ms_space().active_pages.load(Ordering::SeqCst);

        {
            let mut rss_max = self.reserved_pages_max.lock().unwrap();
            if rss > *rss_max {
                *rss_max = rss;
                self.reserved_pages_max_ctr
                    .lock()
                    .unwrap()
                    .set_count(rss.try_into().unwrap());
            }
        }

        {
            let mut rss_sum = self.reserved_pages_all_sum.lock().unwrap();
            let mut rss_total = self.reserved_pages_total.lock().unwrap();

            *rss_sum += rss;
            *rss_total += 1;

            let rss_avg = *rss_sum / *rss_total;
            self.reserved_pages_avg_ctr
                .lock()
                .unwrap()
                .set_count(rss_avg.try_into().unwrap());
        }
    }

    fn set_running(&mut self, running: bool) {
        self.running = running;
    }
}
