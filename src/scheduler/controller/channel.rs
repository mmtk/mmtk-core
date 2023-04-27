use std::collections::VecDeque;

use super::*;

/// A one-way channel for workers to send coordinator packets and notifications to the controller.
struct Channel<VM: VMBinding> {
    sync: Mutex<ChannelSync<VM>>,
    cond: Condvar,
}

/// The synchronized parts of `Channel`.
struct ChannelSync<VM: VMBinding> {
    /// Pending coordinator work packets.
    coordinator_packets: VecDeque<Box<dyn CoordinatorWork<VM>>>,
    /// Whether all workers have parked.
    ///
    /// NOTE: This field is set to `true` by the last parked worker.
    /// It is used to notify the coordinator about the event that all workers have parked.
    /// To resume workers from "group sleeping", use `WorkerMonitor::notify_work_available`.
    all_workers_parked: bool,
}

/// Each worker holds an instance of this.
///
/// It wraps a channel, and only allows workers to access it in expected ways.
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
        sync.coordinator_packets.push_back(work);
        debug!("A worker has sent a coordinator work packet.");
        self.chan.cond.notify_one();
    }

    /// Notify the coordinator that all workers have parked.
    pub fn notify_all_workers_parked(&self) {
        let mut sync = self.chan.sync.lock().unwrap();
        sync.all_workers_parked = true;
        debug!("Notified the coordinator that all workers have parked.");
        self.chan.cond.notify_one();
    }
}

/// The coordinator holds an instance of this.
///
/// It wraps a channel, and only allows the coordinator to access it in expected ways.
pub struct Receiver<VM: VMBinding> {
    chan: Arc<Channel<VM>>,
}

impl<VM: VMBinding> Receiver<VM> {
    /// Get an event.
    pub(super) fn poll_event(&self) -> Event<VM> {
        let mut sync = self.chan.sync.lock().unwrap();
        loop {
            // Make sure the coordinator always sees packets before seeing "all parked".
            if let Some(work) = sync.coordinator_packets.pop_front() {
                debug!("Received a coordinator packet.");
                return Event::Work(work);
            }

            if sync.all_workers_parked {
                debug!("Observed all workers parked.");
                return Event::AllParked;
            }

            sync = self.chan.cond.wait(sync).unwrap();
        }
    }

    /// Reset the "all workers have parked" flag.
    pub fn reset_all_workers_parked(&self) {
        let mut sync = self.chan.sync.lock().unwrap();
        sync.all_workers_parked = false;
        debug!("The all_workers_parked state is reset.");
    }
}

/// This type represents the events the `Receiver` observes.
pub(crate) enum Event<VM: VMBinding> {
    /// Send a work-packet to the coordinator thread.
    Work(Box<dyn CoordinatorWork<VM>>),
    /// Notify the coordinator thread that all GC tasks are finished.
    /// When sending this message, all the work buckets should be
    /// empty, and all the workers should be parked.
    AllParked,
}

/// Create a Sender-Receiver pair.
pub(crate) fn make_channel<VM: VMBinding>() -> (Sender<VM>, Receiver<VM>) {
    let chan = Arc::new(Channel {
        sync: Mutex::new(ChannelSync {
            coordinator_packets: Default::default(),
            all_workers_parked: false,
        }),
        cond: Default::default(),
    });

    let sender = Sender { chan: chan.clone() };
    let receiver = Receiver { chan };
    (sender, receiver)
}
