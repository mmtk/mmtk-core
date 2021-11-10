/// Const funciton for min value of two usize numbers.
pub const fn min_of_usize(a: usize, b: usize) -> usize {
    if a > b {
        b
    } else {
        a
    }
}

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

/// Increment the atomic number by 1. If the number reaches limit, return true and reset the number.
/// Otherwise, return false. This function is thread safe so only the first call that makes the number
/// reach limit will return true.
// clippy suggests to use match new_val.cmp(limit) { Ordering::Greater => ..., Ordering::Equal => ..., Ordering::Less => ... },
// which is unnecessary. Plus clippy lists a known issue with match/ordering that it is slower than if.
#[allow(clippy::comparison_chain)]
pub fn increment_and_check_limit(u: &AtomicUsize, limit: usize) -> bool {
    match u.fetch_update(Ordering::SeqCst, Ordering::Relaxed, |current| {
        // New value after increment
        let new_val = current + 1;
        if new_val < limit {
            // If the new value is smaller than limit, set to the new value.
            Some(new_val)
        } else if new_val == limit {
            // If the new value equals the limit, reset to 0
            Some(0)
        } else {
            // If the new value is larger than the limit, this should not happen as
            // we increment by 1 and we should have reset it to zero when the value ever reaches the limit.
            unreachable!()
        }
    }) {
        // We are not returning None from the update func, so Err is unreachable.
        Err(_) => unreachable!(),
        Ok(old) => old + 1 == limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam::thread;

    #[test]
    fn test_increment_and_check_limit() {
        let counter = AtomicUsize::new(0);
        // Let a large number of threads to compete
        let nthreads = 1000;
        // Limit is the same as thread number so each thread will run exactly once, and the limit is reached exactly once.
        let limit = nthreads;
        let limit_reached = AtomicUsize::new(0);

        thread::scope(|s| {
            let mut threads: Vec<thread::ScopedJoinHandle<()>> = vec![];
            for _ in 0..nthreads {
                threads.push(s.spawn(|_| {
                    // sleep for random duration (0-256ms)
                    std::thread::sleep(std::time::Duration::from_millis(
                        rand::random::<u8>() as u64
                    ));
                    if increment_and_check_limit(&counter, limit) {
                        limit_reached.fetch_add(1, Ordering::SeqCst);
                    }
                }));
            }
            threads.into_iter().for_each(|t| t.join().unwrap());
        })
        .unwrap();
        // We returned true for exactly one thread
        assert_eq!(limit_reached.load(Ordering::SeqCst), 1);
        // The atomic number is exactly 0
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}
