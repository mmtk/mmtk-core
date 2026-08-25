use super::*;
use crate::util::statistics::stats::SharedStats;
use std::sync::Arc;

/// A [`Counter`] that records one latency sample per event (e.g. one per GC pause, for things
/// like time-to-yield or pause time) and reports it as `avg`/`p9999` columns instead of a raw
/// per-phase total.
///
/// This is a thin wrapper around an [`EventCounter`]: it reuses the inner counter's
/// `start`/`stop`/`phase_change` machinery unchanged to accumulate one sample per pause into the
/// per-phase array. It only overrides how the counter is
/// named and printed, bending two parts of the [`Counter`] contract to do so:
///
/// - [`Counter::merge_phases`] always returns `true`.
/// - [`Counter::name`] returns a compound, tab-separated pair of column names (e.g.
///   `"pause-time.avg\tpause-time.p9999"`), so the single `print!("{}\t", c.name())` call site
///   in [`crate::util::statistics::stats::Stats::print_column_names`] prints both headers.
/// - [`Counter::print_total`] prints `"{avg}\t{p9999}"` (two tab-separated values, ignoring the
///   `other` argument), so the single `c.print_total(None)` call site in
///   [`crate::util::statistics::stats::Stats::print_stats`] prints both values.
///
/// This trick only works because `Counter::name()` has a single caller (`Stats`'s own printing
/// code) anywhere in the codebase; if that changes, this would need revisiting.
pub struct LatencySampler {
    inner: EventCounter,
    /// A compound `"{name}.avg\t{name}.p9999"` string, returned by `name()`.
    display_name: String,
}

impl LatencySampler {
    pub fn new(name: &str, stats: Arc<SharedStats>, implicitly_start: bool) -> Self {
        LatencySampler {
            inner: EventCounter::new(name.to_string(), stats, implicitly_start, false),
            display_name: format!("{name}.avg\t{name}.p9999"),
        }
    }

    /// Record one latency sample (e.g. a duration in nanoseconds).
    pub fn record(&mut self, value: u64) {
        self.inner.inc_by(value);
    }

    /// The recorded samples, one per pause. Every sample is recorded during the STW phase (see
    /// `record`), so only the odd-indexed phase counts hold real values; the even-indexed
    /// (mutator-phase) ones are always 0 and are skipped here.
    fn samples(&self) -> Vec<u64> {
        self.inner
            .count
            .iter()
            .skip(1)
            .step_by(2)
            .copied()
            .collect()
    }
}

impl Counter for LatencySampler {
    fn start(&mut self) {
        self.inner.start();
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn phase_change(&mut self, old_phase: usize) {
        self.inner.phase_change(old_phase);
    }

    fn print_count(&self, phase: usize) {
        self.inner.print_count(phase);
    }

    fn get_total(&self, other: Option<bool>) -> u64 {
        self.inner.get_total(other)
    }

    fn print_total(&self, _other: Option<bool>) {
        let mut samples = self.samples();
        if samples.is_empty() {
            print!("n/a\tn/a");
            return;
        }
        let avg_ns = samples.iter().sum::<u64>() as f64 / samples.len() as f64;
        // Exact p9999 (99.99th percentile), computed by sorting all samples and using the
        // nearest-rank method. Note that with fewer than 10,000 samples, this is guaranteed to
        // just return the maximum.
        samples.sort_unstable();
        let rank = ((99.99 / 100.0) * samples.len() as f64).ceil() as usize;
        let p9999_ns = samples[rank.clamp(1, samples.len()) - 1];
        print!("{:.2}\t{:.2}", avg_ns / 1e6, p9999_ns as f64 / 1e6);
    }

    fn print_min(&self, other: bool) {
        self.inner.print_min(other);
    }

    fn print_max(&self, other: bool) {
        self.inner.print_max(other);
    }

    fn print_last(&self) {
        self.inner.print_last();
    }

    fn merge_phases(&self) -> bool {
        true
    }

    fn implicitly_start(&self) -> bool {
        self.inner.implicitly_start()
    }

    fn name(&self) -> &String {
        &self.display_name
    }
}
