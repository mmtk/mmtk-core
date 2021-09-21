use crate::util::constants::DEFAULT_STRESS_FACTOR;
use crate::util::constants::LOG_BYTES_IN_MBYTE;
use std::cell::UnsafeCell;
use std::default::Default;
use std::ops::Deref;
use std::str::FromStr;

custom_derive! {
    #[derive(Copy, Clone, EnumFromStr, Debug)]
    pub enum NurseryZeroingOptions {
        Temporal,
        Nontemporal,
        Concurrent,
        Adaptive,
    }
}

custom_derive! {
    #[derive(Copy, Clone, EnumFromStr, Debug)]
    pub enum PlanSelector {
        NoGC,
        SemiSpace,
        GenCopy,
        GenImmix,
        MarkSweep,
        PageProtect,
        Immix,
    }
}

/// MMTk option for perf events
///
/// The format is
/// ```
/// <event> ::= <event-name> "," <pid> "," <cpu>
/// <events> ::= <event> ";" <events> | <event> | ""
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerfEventOptions {
    pub events: Vec<(String, i32, i32)>,
}

impl PerfEventOptions {
    fn parse_perf_events(events: &str) -> Result<Vec<(String, i32, i32)>, String> {
        events
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| {
                let e: Vec<&str> = e.split(',').into_iter().collect();
                if e.len() != 3 {
                    Err("Please supply (event name, pid, cpu)".into())
                } else {
                    let event_name = e[0].into();
                    let pid = e[1]
                        .parse()
                        .map_err(|_| String::from("Failed to parse cpu"))?;
                    let cpu = e[2]
                        .parse()
                        .map_err(|_| String::from("Failed to parse cpu"))?;
                    Ok((event_name, pid, cpu))
                }
            })
            .collect()
    }
}

impl FromStr for PerfEventOptions {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        PerfEventOptions::parse_perf_events(s).map(|events| PerfEventOptions { events })
    }
}

/// The default nursery space size.
pub const NURSERY_SIZE: usize = 32 << LOG_BYTES_IN_MBYTE;
/// The default min nursery size. This can be set through command line options.
/// This does not affect the actual space we create as nursery. It is only used in GC trigger check.
pub const DEFAULT_MIN_NURSERY: usize = 32 << LOG_BYTES_IN_MBYTE;
/// The default max nursery size. This can be set through command line options.
/// This does not affect the actual space we create as nursery. It is only used in GC trigger check.
pub const DEFAULT_MAX_NURSERY: usize = 32 << LOG_BYTES_IN_MBYTE;

pub struct UnsafeOptionsWrapper(UnsafeCell<Options>);

// TODO: We should carefully examine the unsync with UnsafeCell. We should be able to provide a safe implementation.
unsafe impl Sync for UnsafeOptionsWrapper {}

impl UnsafeOptionsWrapper {
    pub const fn new(o: Options) -> UnsafeOptionsWrapper {
        UnsafeOptionsWrapper(UnsafeCell::new(o))
    }
    /// # Safety
    /// This method is not thread safe, as internally it acquires a mutable reference to self.
    /// It is supposed to be used by one thread during boot time.
    pub unsafe fn process(&self, name: &str, value: &str) -> bool {
        (&mut *self.0.get()).set_from_camelcase_str(name, value)
    }
}
impl Deref for UnsafeOptionsWrapper {
    type Target = Options;
    fn deref(&self) -> &Options {
        unsafe { &*self.0.get() }
    }
}

fn always_valid<T>(_: &T) -> bool {
    true
}

macro_rules! options {
    ($($name:ident: $type:ty[$validator:expr] = $default:expr),*,) => [
        options!($($name: $type[$validator] = $default),*);
    ];
    ($($name:ident: $type:ty[$validator:expr] = $default:expr),*) => [
        pub struct Options {
            $(pub $name: $type),*
        }
        impl Options {
            pub fn set_from_str(&mut self, s: &str, val: &str)->bool {
                match s {
                    // Parse the given value from str (by env vars or by calling process()) to the right type
                    $(stringify!($name) => if let Ok(ref val) = val.parse::<$type>() {
                        // Validate
                        let validate_fn = $validator;
                        let is_valid = validate_fn(val);
                        if is_valid {
                            // Only set value if valid.
                            self.$name = val.clone();
                        } else {
                            eprintln!("Warn: unable to set {}={:?}. Invalid value. Default value will be used.", s, val);
                        }
                        is_valid
                    } else {
                        eprintln!("Warn: unable to set {}={:?}. Cant parse value. Default value will be used.", s, val);
                        false
                    })*
                    _ => panic!("Invalid Options key")
                }
            }
        }
        impl Default for Options {
            fn default() -> Self {
                let mut options = Options {
                    $($name: $default),*
                };

                // If we have env vars that start with MMTK_ and match any option (such as MMTK_STRESS_FACTOR),
                // we set the option to its value (if it is a valid value). Otherwise, use the default value.
                const PREFIX: &str = "MMTK_";
                for (key, val) in std::env::vars() {
                    // strip the prefix, and get the lower case string
                    if let Some(rest_of_key) = key.strip_prefix(PREFIX) {
                        let lowercase: &str = &rest_of_key.to_lowercase();
                        match lowercase {
                            $(stringify!($name) => { options.set_from_str(lowercase, &val); },)*
                            _ => {}
                        }
                    }
                }
                return options;
            }
        }
    ]
}
options! {
    // The plan to use. This needs to be initialized before creating an MMTk instance (currently by setting env vars)
    plan:                  PlanSelector         [always_valid] = PlanSelector::NoGC,
    // Number of GC threads.
    threads:               usize                [|v: &usize| *v > 0]    = num_cpus::get(),
    // Enable an optimization that only scans the part of the stack that has changed since the last GC (not supported)
    use_short_stack_scans: bool                 [always_valid] = false,
    // Enable a return barrier (not supported)
    use_return_barrier:    bool                 [always_valid] = false,
    // Should we eagerly finish sweeping at the start of a collection? (not supported)
    eager_complete_sweep:  bool                 [always_valid] = false,
    // Should we ignore GCs requested by the user (e.g. java.lang.System.gc)?
    ignore_system_g_c:     bool                 [always_valid] = false,
    // The upper bound of nursery size. This needs to be initialized before creating an MMTk instance (currently by setting env vars)
    max_nursery:           usize                [|v: &usize| *v > 0 ] = DEFAULT_MAX_NURSERY,
    // The lower bound of nusery size. This needs to be initialized before creating an MMTk instance (currently by setting env vars)
    min_nursery:           usize                [|v: &usize| *v > 0 ] = DEFAULT_MIN_NURSERY,
    // Should a major GC be performed when a system GC is required?
    full_heap_system_gc:   bool                 [always_valid] = false,
    // Should we shrink/grow the heap to adjust to application working set? (not supported)
    variable_size_heap:    bool                 [always_valid] = true,
    // Should finalization be disabled?
    no_finalizer:          bool                 [always_valid] = false,
    // Should reference type processing be disabled?
    no_reference_types:    bool                 [always_valid] = false,
    // The zeroing approach to use for new object allocations. Affects each plan differently. (not supported)
    nursery_zeroing:       NurseryZeroingOptions[always_valid] = NurseryZeroingOptions::Temporal,
    // How frequent (every X bytes) should we do a stress GC?
    stress_factor:         usize                [always_valid] = DEFAULT_STRESS_FACTOR,
    // How frequent (every X bytes) should we run analysis (a STW event that collects data)
    analysis_factor:       usize                [always_valid] = DEFAULT_STRESS_FACTOR,
    // The size of vmspace. This needs to be initialized before creating an MMTk instance (currently by setting env vars)
    // FIXME: This value is set for JikesRVM. We need a proper way to set options.
    //   We need to set these values programmatically in VM specific code.
    vm_space_size:         usize                [|v: &usize| *v > 0]    = 0x7cc_cccc,
    // Perf events to measure
    // Semicolons are used to separate events
    // Each event is in the format of event_name,pid,cpu (see man perf_event_open for what pid and cpu mean)
    //
    // Measuring perf events for work packets
    work_perf_events:       PerfEventOptions     [always_valid] = PerfEventOptions {events: vec![]},
    // Measuring perf events for GC and mutators
    phase_perf_events:      PerfEventOptions     [always_valid] = PerfEventOptions {events: vec![]}
}

impl Options {
    fn set_from_camelcase_str(&mut self, s: &str, val: &str) -> bool {
        trace!("Trying to process option pair: ({}, {})", s, val);

        let mut sr = String::with_capacity(s.len());
        for c in s.chars() {
            if c.is_uppercase() {
                sr.push('_');
                for c in c.to_lowercase() {
                    sr.push(c);
                }
            } else {
                sr.push(c)
            }
        }

        let result = self.set_from_str(sr.as_str(), val);

        trace!("Trying to process option pair: ({})", sr);

        if result {
            trace!("Validation passed");
        } else {
            trace!("Validation failed")
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::constants::DEFAULT_STRESS_FACTOR;
    use crate::util::options::Options;
    use crate::util::test_util::{serial_test, with_cleanup};

    #[test]
    fn no_env_var() {
        serial_test(|| {
            let options = Options::default();
            assert_eq!(options.stress_factor, DEFAULT_STRESS_FACTOR);
        })
    }

    #[test]
    fn with_valid_env_var() {
        serial_test(|| {
            with_cleanup(
                || {
                    std::env::set_var("MMTK_STRESS_FACTOR", "4096");

                    let options = Options::default();
                    assert_eq!(options.stress_factor, 4096);
                },
                || {
                    std::env::remove_var("MMTK_STRESS_FACTOR");
                },
            )
        })
    }

    #[test]
    fn with_multiple_valid_env_vars() {
        serial_test(|| {
            with_cleanup(
                || {
                    std::env::set_var("MMTK_STRESS_FACTOR", "4096");
                    std::env::set_var("MMTK_NO_FINALIZER", "true");

                    let options = Options::default();
                    assert_eq!(options.stress_factor, 4096);
                    assert!(options.no_finalizer);
                },
                || {
                    std::env::remove_var("MMTK_STRESS_FACTOR");
                    std::env::remove_var("MMTK_NO_FINALIZER");
                },
            )
        })
    }

    #[test]
    fn with_invalid_env_var_value() {
        serial_test(|| {
            with_cleanup(
                || {
                    // invalid value, we cannot parse the value, so use the default value
                    std::env::set_var("MMTK_STRESS_FACTOR", "abc");

                    let options = Options::default();
                    assert_eq!(options.stress_factor, DEFAULT_STRESS_FACTOR);
                },
                || {
                    std::env::remove_var("MMTK_STRESS_FACTOR");
                },
            )
        })
    }

    #[test]
    fn with_invalid_env_var_key() {
        serial_test(|| {
            with_cleanup(
                || {
                    // invalid value, we cannot parse the value, so use the default value
                    std::env::set_var("MMTK_ABC", "42");

                    let options = Options::default();
                    assert_eq!(options.stress_factor, DEFAULT_STRESS_FACTOR);
                },
                || {
                    std::env::remove_var("MMTK_ABC");
                },
            )
        })
    }

    #[test]
    fn test_str_option_default() {
        serial_test(|| {
            let options = Options::default();
            assert_eq!(
                &options.work_perf_events,
                &PerfEventOptions { events: vec![] }
            );
        })
    }

    #[test]
    fn test_str_option_from_env_var() {
        serial_test(|| {
            with_cleanup(
                || {
                    std::env::set_var("MMTK_WORK_PERF_EVENTS", "PERF_COUNT_HW_CPU_CYCLES,0,-1");

                    let options = Options::default();
                    assert_eq!(
                        &options.work_perf_events,
                        &PerfEventOptions {
                            events: vec![("PERF_COUNT_HW_CPU_CYCLES".into(), 0, -1)]
                        }
                    );
                },
                || {
                    std::env::remove_var("MMTK_WORK_PERF_EVENTS");
                },
            )
        })
    }

    #[test]
    fn test_invalid_str_option_from_env_var() {
        serial_test(|| {
            with_cleanup(
                || {
                    // The option needs to start with "hello", otherwise it is invalid.
                    std::env::set_var("MMTK_WORK_PERF_EVENTS", "PERF_COUNT_HW_CPU_CYCLES");

                    let options = Options::default();
                    // invalid value from env var, use default.
                    assert_eq!(
                        &options.work_perf_events,
                        &PerfEventOptions { events: vec![] }
                    );
                },
                || {
                    std::env::remove_var("MMTK_WORK_PERF_EVENTS");
                },
            )
        })
    }
}
