use super::worker::*;
use crate::plan::tracing::gc_work::root::DefaultRootsWorkFactory;
use crate::vm::{RootsWorkFactory, VMBinding};
use crate::{mmtk::MMTK, plan::tracing::Trace};
#[cfg(feature = "work_packet_stats")]
use std::any::{type_name, TypeId};

/// Diagnostic: per-pause wall time attributed to each work packet type, enabled by
/// `MMTK_LXR_STATS`. `work_packet_stats` exists for this but only reports at harness end,
/// which cannot answer "what is this one pause spending its time on".
pub mod packet_timing {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    pub fn enabled() -> bool {
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("MMTK_LXR_STATS").is_some())
    }

    fn table() -> &'static Mutex<HashMap<&'static str, (u64, u128)>> {
        static T: OnceLock<Mutex<HashMap<&'static str, (u64, u128)>>> = OnceLock::new();
        T.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub fn record(name: &'static str, nanos: u128) {
        let mut t = table().lock().unwrap();
        let e = t.entry(name).or_insert((0, 0));
        e.0 += 1;
        e.1 += nanos;
    }

    /// Discard everything recorded so far.
    ///
    /// Called when the world stops, so the table describes only this pause. Without it the table
    /// spanned from the previous `resume_mutators`, absorbing all the concurrent work executed
    /// during the intervening mutator window -- which is how entries came to report far more time
    /// than the pause they were printed under.
    pub fn reset() {
        if !enabled() {
            return;
        }
        table().lock().unwrap().clear();
    }

    /// Returns the types that accounted for the most time since the last call, and clears.
    pub fn take_top(n: usize) -> Vec<(&'static str, u64, u128)> {
        let mut t = table().lock().unwrap();
        let mut v: Vec<_> = t.iter().map(|(k, (c, ns))| (*k, *c, *ns)).collect();
        t.clear();
        v.sort_by(|a, b| b.2.cmp(&a.2));
        v.truncate(n);
        v
    }
}

/// Diagnostic wall-clock timeline of a single pause.
///
/// [`packet_timing`] sums CPU across all workers, so it cannot say where a pause's *wall* time
/// goes: a 1ms pause can contain 30ms of packet time, and concurrent work executed during the
/// preceding mutator window lands in the same table. This records ordered marks against one clock
/// instead, so the gap between consecutive marks is real elapsed time on the critical path --
/// including the park/wake handshake between work-bucket stages, which no packet timer sees.
pub mod stage_timeline {
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    pub fn enabled() -> bool {
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("MMTK_LXR_TIMELINE").is_some())
    }

    #[allow(clippy::type_complexity)]
    fn state() -> &'static Mutex<(Option<Instant>, Vec<(String, u128)>)> {
        static T: OnceLock<Mutex<(Option<Instant>, Vec<(String, u128)>)>> = OnceLock::new();
        T.get_or_init(|| Mutex::new((None, Vec::new())))
    }

    /// Begin a new pause timeline.  Called once the world has stopped.
    pub fn start() {
        if !enabled() {
            return;
        }
        let mut s = state().lock().unwrap();
        s.0 = Some(Instant::now());
        s.1.clear();
    }

    /// Record a named point on the critical path.
    pub fn mark(label: impl Into<String>) {
        if !enabled() {
            return;
        }
        let mut s = state().lock().unwrap();
        if let Some(t0) = s.0 {
            let ns = t0.elapsed().as_nanos();
            s.1.push((label.into(), ns));
        }
    }

    /// Drain the timeline recorded since [`start`].
    pub fn take() -> Vec<(String, u128)> {
        if !enabled() {
            return Vec::new();
        }
        let mut s = state().lock().unwrap();
        std::mem::take(&mut s.1)
    }
}

/// This defines a GC work packet which are assigned to the [`GCWorker`]s by the scheduler.
/// Work packets carry payloads that indicate the work to be done. For example, a work packet may
/// contain a pointer to a stack that must be scanned, or it may contain a large buffer of pointers
/// that need to be traced, or it might contain a range of static variables to be scanned, etc. The size
/// of the work packet will need to consider at least two points of tension: the work packet must be large
/// enough to ensure that the costs of managing the work packets do not dominate, and the packet must be
/// small enough that good load balancing is achieved.
pub trait GCWork<VM: VMBinding>: 'static + Send {
    /// Define the work for this packet. However, this is not supposed to be called directly.
    /// Usually `do_work_with_stat()` should be used.
    ///
    /// Most work packets are polled and executed in the worker's main loop ([`GCWorker::run`])
    /// using `do_work_with_stat`.  If `do_work` is called directly during the execution of another
    /// work packet, bypassing `do_work_with_stat()`, this work packet will not be counted into the
    /// number of work packets executed, and the execution time of this work packet will be counted
    /// as part of the execution time of the other work packet.  Only call this method directly if
    /// this is what you intend.  But you should always consider adding the work packet
    /// into a bucket so that other GC workers can execute it in parallel, unless the context-
    /// switching overhead is a problem.
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>);

    /// Do work and collect statistics. This internally calls `do_work()`. In most cases,
    /// this should be called rather than `do_work()` so that MMTk can correctly collect
    /// statistics for the work packets.
    /// If the feature "work_packet_stats" is not enabled, this call simply forwards the call
    /// to `do_work()`.
    fn do_work_with_stat(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        debug!("{}", std::any::type_name::<Self>());
        debug_assert!(!worker.tls.0.0.is_null(), "TLS must be set correctly for a GC worker before the worker does any work. GC Worker {} has no valid tls.", worker.ordinal);

        #[cfg(feature = "work_packet_stats")]
        // Start collecting statistics
        let stat = {
            let mut worker_stat = worker.shared.borrow_stat_mut();
            worker_stat.measure_work(TypeId::of::<Self>(), type_name::<Self>(), mmtk)
        };

        // Do the actual work
        if packet_timing::enabled() {
            let t = std::time::Instant::now();
            self.do_work(worker, mmtk);
            packet_timing::record(std::any::type_name::<Self>(), t.elapsed().as_nanos());
        } else {
            self.do_work(worker, mmtk);
        }

        #[cfg(feature = "work_packet_stats")]
        // Finish collecting statistics
        {
            let mut worker_stat = worker.shared.borrow_stat_mut();
            stat.end_of_work(&mut worker_stat);
        }
    }

    /// Get the compile-time static type name for the work packet.
    fn get_type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

use crate::plan::Plan;

/// This trait provides a group of associated types that are needed to
/// create GC work packets for a certain plan. For example, `GCWorkScheduler.schedule_common_work()`
/// needs this trait to schedule different work packets. For certain plans,
/// they may need to provide several types that implement this trait, e.g. one for
/// nursery GC, one for mature GC.
///
/// Note: Because `GCWorkContext` is often used as parameters of implementations of `GCWork`, we
/// let GCWorkContext require `Send + 'static`.  Since `GCWorkContext` is just a group of
/// associated types, its implementations should not have any actual fields other than
/// `PhantomData`, and will automatically have `Send + 'static`.
pub trait GCWorkContext: Send + 'static {
    type VM: VMBinding;
    type PlanType: Plan<VM = Self::VM>;

    // FIXME: We should use `SFTTrace` as the default value for `DefaultTrace`, and
    // `UnsupportedTrace` for `PinningTrace`.  However, this requires `associated_type_defaults`
    // which has not yet been stablized. See: https://github.com/rust-lang/rust/issues/29661

    /// The [`Trace`] implementation to use for tracing edges that do not have special pinning
    /// requirements.  Concrete plans and spaces may choose to move or not to move the objects the
    /// traced edges point to.
    type DefaultTrace: Trace<VM = Self::VM>;

    /// The [`Trace`] implementation to use for tracing edges that must not be updated (i.e. the
    /// objects the traced edges pointed to must not be moved).  This is used for implementing
    /// pinning roots and transitive pinning roots.
    ///
    /// -   For non-transitive pinning roots, [`Self::PinningTrace`] will be used to trace the edges
    ///     from roots to objects, but their descendents will be traced using
    ///     [`Self::DefaultTrace`].
    /// -   For transitive pinning roots, [`Self::PinningTrace`] will be used to trace the edges
    ///     from roots to objects, and will also be used to trace the outgoing edges of all objects
    ///     reachable from transitive pinning roots.
    ///
    /// If a plan does not support object pinning, it should use [`UnsupportedTrace`] for this type
    /// member.
    ///
    /// [`UnsupportedTrace`]: crate::plan::tracing::UnsupportedTrace
    type PinningTrace: Trace<VM = Self::VM>;

    /// Create an instance of [`RootsWorkFactory`] for root scanning in the current GC.
    ///
    /// The default implementation creates [`DefaultRootsWorkFactory`] which is sufficient for
    /// stop-the-world tracing GC.  Plans that need custom [`RootsWorkFactory`] implementations can
    /// override this method.
    fn make_roots_work_factory(
        mmtk: &'static MMTK<Self::VM>,
    ) -> impl RootsWorkFactory<<Self::VM as VMBinding>::VMSlot> {
        DefaultRootsWorkFactory::<Self::VM, Self::DefaultTrace, Self::PinningTrace>::new(mmtk)
    }
}
