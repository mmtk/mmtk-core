use super::*;

/// The synchronized parts of `Channel`.
struct ChannelSync<VM: VMBinding> {
    coordinator_packets: Vec<Box<dyn CoordinatorWork<VM>>>,
    all_workers_parked: bool,
}

/// A one-way channel for workers to send coordinator packets and notifications to the controller.
struct Channel<VM: VMBinding> {
    sync: Mutex<ChannelSync<VM>>,
    cond: Condvar,
}

/// Each worker holds an instance of this, mainly for access control.
pub struct Sender<VM: VMBinding> {
    chan: Arc<Channel<VM>>,
}

impl<VM: VMBinding> Clone for Sender<VM> {
    fn clone(&self) -> Self {
        Self {
            chan: self.chan.clone(),
        }
    }
}

impl<VM: VMBinding> Sender<VM> {
    /// Send a coordinator work packet to the coordinator.
    pub fn add_coordinator_work(&self, work: Box<dyn CoordinatorWork<VM>>) {
        let mut sync = self.chan.sync.lock().unwrap();
        sync.coordinator_packets.push(work);
        debug!("Submitted coordinator work!");
        self.chan.cond.notify_one();
    }

    /// Notify that all workers have parked.
    pub fn notify_all_workers_parked(&self) {
        let mut sync = self.chan.sync.lock().unwrap();
        sync.all_workers_parked = true;
        debug!("Notified all workers parked!");
        self.chan.cond.notify_one();
    }
}

/// The coordinator holds an instance of this, mainly for access control.
pub struct Receiver<VM: VMBinding> {
    chan: Arc<Channel<VM>>,
}

impl<VM: VMBinding> Receiver<VM> {
    /// Get an event.
    pub(super) fn poll_event(&self) -> WorkerToControllerEvent<VM> {
        let mut sync = self.chan.sync.lock().unwrap();
        loop {
            // Make sure the coordinator always sees packets before seeing "all parked".
            if let Some(work) = sync.coordinator_packets.pop() {
                debug!("Received coordinator packet.");
                return WorkerToControllerEvent::Work(work);
            }

            if sync.all_workers_parked {
                debug!("Observed all workers parked.");
                return WorkerToControllerEvent::AllParked;
            }

            sync = self.chan.cond.wait(sync).unwrap();
        }
    }

    /// Reset the "all workers have parked" flag.
    pub fn reset_all_workers_parked(&self) {
        let mut sync = self.chan.sync.lock().unwrap();
        sync.all_workers_parked = false;
        debug!("All-workers-parked state reset.");
    }
}

/// The receiver will generate this event type.
pub(crate) enum WorkerToControllerEvent<VM: VMBinding> {
    /// Send a work-packet to the coordinator thread/
    Work(Box<dyn CoordinatorWork<VM>>),
    /// Notify the coordinator thread that all GC tasks are finished.
    /// When sending this message, all the work buckets should be
    /// empty, and all the workers should be parked.
    AllParked,
}

/// Create a Sender-Receiver pair.
pub(crate) fn make_channel<VM: VMBinding>() -> (Sender<VM>, Receiver<VM>) {
    let w2c = Arc::new(Channel {
        sync: Mutex::new(ChannelSync {
            coordinator_packets: Default::default(),
            all_workers_parked: false,
        }),
        cond: Default::default(),
    });

    let worker_end = Sender { chan: w2c.clone() };
    let controller_end = Receiver { chan: w2c };
    (worker_end, controller_end)
}
