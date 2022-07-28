use crate::util::constants::DEFAULT_STRESS_FACTOR;
use crate::util::constants::LOG_BYTES_IN_MBYTE;
use std::default::Default;
use std::fmt::Debug;
use std::str::FromStr;
use strum_macros::EnumString;

#[derive(Copy, Clone, EnumString, Debug)]
pub enum NurseryZeroingOptions {
    Temporal,
    Nontemporal,
    Concurrent,
    Adaptive,
}

#[derive(Copy, Clone, EnumString, Debug)]
pub enum PlanSelector {
    NoGC,
    SemiSpace,
    GenCopy,
    GenImmix,
    PageProtect,
    Immix,
    MarkCompact,
    MarkSweep,
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

fn always_valid<T>(_: &T) -> bool {
    true
}

/// An MMTk option of a given type.
/// This type allows us to store some metadata for the option. To get the value of an option,
/// you can simply dereference it (for example, *options.threads).
#[derive(Clone)]
pub struct MMTKOption<T: Debug + Clone> {
    /// The actual value for the option
    value: T,
    /// The validator to ensure the value is valid.
    validator: fn(&T) -> bool,
    /// Can we set this option through env vars?
    from_env_var: bool,
    /// Can we set this option through command line options/API?
    from_command_line: bool,
}

impl<T: Debug + Clone> MMTKOption<T> {
    /// Create a new MMTKOption
    pub fn new(
        value: T,
        validator: fn(&T) -> bool,
        from_env_var: bool,
        from_command_line: bool,
    ) -> Self {
        // FIXME: We should enable the following check to make sure the initial value is valid.
        // However, we cannot enable it now. For options like perf events, the validator checks
        // if the perf event feature is enabled. So when the perf event features are not enabled,
        // the validator will fail whatever value we try to set (including the initial value).
        // Ideally, we conditionally compile options based on the feature. But options! marcro
        // does not allow attributes in it, so we cannot conditionally compile options.
        // let is_valid = validator(&value);
        // assert!(
        //     is_valid,
        //     "Unable to create MMTKOption: initial value {:?} is invalid",
        //     value
        // );
        MMTKOption {
            value,
            validator,
            from_env_var,
            from_command_line,
        }
    }

    /// Set the option to the given value. Returns true if the value is valid, and we set the option to the value.
    pub fn set(&mut self, value: T) -> bool {
        if (self.validator)(&value) {
            self.value = value;
            return true;
        }
        false
    }
}

// Dereference an option to get its value.
impl<T: Debug + Clone> std::ops::Deref for MMTKOption<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

macro_rules! options {
    // Verify whether we can set an option through env var or command line.
    (@verify_set_from($self: expr, $key: expr, $verify_field: ident, $($name: ident),*)) => {
        match $key {
            $(stringify!($name) => { assert!($self.$name.$verify_field, "cannot set option {} (not {})", $key, stringify!($verify_field)) }),*
            _ => panic!("Invalid Options key: {}", $key)
        }
    };

    ($($name:ident: $type:ty[env_var: $env_var:expr, command_line: $command_line:expr][$validator:expr] = $default:expr),*,) => [
        options!($($name: $type[env_var: $env_var, command_line: $command_line, mutable: $mutable][$validator] = $default),*);
    ];
    ($($name:ident: $type:ty[env_var: $env_var:expr, command_line: $command_line:expr][$validator:expr] = $default:expr),*) => [
        #[derive(Clone)]
        pub struct Options {
            $(pub $name: MMTKOption<$type>),*
        }
        impl Options {
            /// Set an option from env var
            pub fn set_from_env_var(&mut self, s: &str, val: &str) -> bool {
                options!(@verify_set_from(self, s, from_env_var, $($name),*));
                self.set_inner(s, val)
            }

            /// Set an option from command line
            pub fn set_from_command_line(&mut self, s: &str, val: &str) -> bool {
                options!(@verify_set_from(self, s, from_command_line, $($name),*));
                self.set_inner(s, val)
            }

            /// Bulk process options. Returns true if all the options are processed successfully.
            /// This method returns false if the option string is invalid, or if it includes any invalid option.
            ///
            /// Arguments:
            /// * `options`: a string that is key value pairs separated by white spaces, e.g. "threads=1 stress_factor=4096"
            pub fn set_bulk_from_command_line(&mut self, options: &str) -> bool {
                for opt in options.split_ascii_whitespace() {
                    let kv_pair: Vec<&str> = opt.split('=').collect();
                    if kv_pair.len() != 2 {
                        return false;
                    }

                    let key = kv_pair[0];
                    let val = kv_pair[1];
                    if !self.set_from_command_line(key, val) {
                        return false;
                    }
                }

                true
            }

            /// Set an option and run its validator for its value.
            fn set_inner(&mut self, s: &str, val: &str)->bool {
                match s {
                    // Parse the given value from str (by env vars or by calling process()) to the right type
                    $(stringify!($name) => if let Ok(typed_val) = val.parse::<$type>() {
                        let is_set = self.$name.set(typed_val);
                        if !is_set {
                            eprintln!("Warn: unable to set {}={:?}. Invalid value. Default value will be used.", s, val);
                        }
                        is_set
                    } else {
                        eprintln!("Warn: unable to set {}={:?}. Cant parse value. Default value will be used.", s, val);
                        false
                    })*
                    _ => panic!("Invalid Options key: {}", s)
                }
            }
        }
        impl Default for Options {
            fn default() -> Self {
                let mut options = Options {
                    $($name: MMTKOption::new($default, $validator, $env_var,$command_line)),*
                };

                // If we have env vars that start with MMTK_ and match any option (such as MMTK_STRESS_FACTOR),
                // we set the option to its value (if it is a valid value). Otherwise, use the default value.
                const PREFIX: &str = "MMTK_";
                for (key, val) in std::env::vars() {
                    // strip the prefix, and get the lower case string
                    if let Some(rest_of_key) = key.strip_prefix(PREFIX) {
                        let lowercase: &str = &rest_of_key.to_lowercase();
                        match lowercase {
                            $(stringify!($name) => { options.set_from_env_var(lowercase, &val); },)*
                            _ => {}
                        }
                    }
                }
                return options;
            }
        }
    ]
}

// Currently we allow all the options to be set by env var for the sake of convenience.
// At some point, we may disallow this and all the options can only be set by command line.
options! {
    // The plan to use.
    plan:                  PlanSelector         [env_var: true, command_line: true] [always_valid] = PlanSelector::NoGC,
    // Number of GC worker threads. (There is always one GC controller thread.)
    // FIXME: Currently we create GCWorkScheduler when MMTK is created, which is usually static.
    // To allow this as a command-line option, we need to refactor the creation fo the `MMTK` instance.
    // See: https://github.com/mmtk/mmtk-core/issues/532
    threads:               usize                [env_var: true, command_line: true] [|v: &usize| *v > 0]    = num_cpus::get(),
    // Heap size. Default to 512MB.
    // TODO: We should have a default heap size related to the max physical memory.
    heap_size:             usize                [env_var: true, command_line: true] [|v: &usize| *v > 0]    = 512 << 20,
    // Enable an optimization that only scans the part of the stack that has changed since the last GC (not supported)
    use_short_stack_scans: bool                 [env_var: true, command_line: true]  [always_valid] = false,
    // Enable a return barrier (not supported)
    use_return_barrier:    bool                 [env_var: true, command_line: true]  [always_valid] = false,
    // Should we eagerly finish sweeping at the start of a collection? (not supported)
    eager_complete_sweep:  bool                 [env_var: true, command_line: true]  [always_valid] = false,
    // Should we ignore GCs requested by the user (e.g. java.lang.System.gc)?
    ignore_system_g_c:     bool                 [env_var: true, command_line: true]  [always_valid] = false,
    // The upper bound of nursery size.
    max_nursery:           usize                [env_var: true, command_line: true]  [|v: &usize| *v > 0 ] = DEFAULT_MAX_NURSERY,
    // The lower bound of nusery size.
    min_nursery:           usize                [env_var: true, command_line: true]  [|v: &usize| *v > 0 ] = DEFAULT_MIN_NURSERY,
    // Should a major GC be performed when a system GC is required?
    full_heap_system_gc:   bool                 [env_var: true, command_line: true]  [always_valid] = false,
    // Should we shrink/grow the heap to adjust to application working set? (not supported)
    variable_size_heap:    bool                 [env_var: true, command_line: true]  [always_valid] = true,
    // Should finalization be disabled?
    no_finalizer:          bool                 [env_var: true, command_line: true]  [always_valid] = false,
    // Should reference type processing be disabled?
    // If reference type processing is disabled, no weak reference processing work is scheduled,
    // and we expect a binding to treat weak references as strong references.
    // We disable weak reference processing by default, as we are still working on it. This will be changed to `false`
    // once weak reference processing is implemented properly.
    no_reference_types:    bool                 [env_var: true, command_line: true]  [always_valid] = true,
    // The zeroing approach to use for new object allocations. Affects each plan differently. (not supported)
    nursery_zeroing:       NurseryZeroingOptions[env_var: true, command_line: true]  [always_valid] = NurseryZeroingOptions::Temporal,
    // How frequent (every X bytes) should we do a stress GC?
    stress_factor:         usize                [env_var: true, command_line: true]  [always_valid] = DEFAULT_STRESS_FACTOR,
    // How frequent (every X bytes) should we run analysis (a STW event that collects data)
    analysis_factor:       usize                [env_var: true, command_line: true]  [always_valid] = DEFAULT_STRESS_FACTOR,
    // Precise stress test. Trigger stress GCs exactly at X bytes if this is true. This is usually used to test the GC correctness
    // and will significantly slow down the mutator performance. If this is false, stress GCs will only be triggered when an allocation reaches
    // the slow path. This means we may have allocated more than X bytes or fewer than X bytes when we actually trigger a stress GC.
    // But this should have no obvious mutator overhead, and can be used to test GC performance along with a larger stress
    // factor (e.g. tens of metabytes).
    precise_stress:        bool                 [env_var: true, command_line: true]  [always_valid] = true,
    // The size of vmspace.
    // FIXME: This value is set for JikesRVM. We need a proper way to set options.
    //   We need to set these values programmatically in VM specific code.
    vm_space_size:         usize                [env_var: true, command_line: true] [|v: &usize| *v > 0]    = 0x7cc_cccc,
    // Perf events to measure
    // Semicolons are used to separate events
    // Each event is in the format of event_name,pid,cpu (see man perf_event_open for what pid and cpu mean).
    // For example, PERF_COUNT_HW_CPU_CYCLES,0,-1 measures the CPU cycles for the current process on all the CPU cores.
    //
    // Measuring perf events for work packets. NOTE that be VERY CAREFUL when using this option, as this may greatly slowdown GC performance.
    // TODO: Ideally this option should only be included when the features 'perf_counter' and 'work_packet_stats' are enabled. The current macro does not allow us to do this.
    work_perf_events:       PerfEventOptions     [env_var: true, command_line: true] [|_| cfg!(all(feature = "perf_counter", feature = "work_packet_stats"))] = PerfEventOptions {events: vec![]},
    // Measuring perf events for GC and mutators
    // TODO: Ideally this option should only be included when the features 'perf_counter' are enabled. The current macro does not allow us to do this.
    phase_perf_events:      PerfEventOptions     [env_var: true, command_line: true] [|_| cfg!(feature = "perf_counter")] = PerfEventOptions {events: vec![]}
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
            assert_eq!(*options.stress_factor, DEFAULT_STRESS_FACTOR);
        })
    }

    #[test]
    fn with_valid_env_var() {
        serial_test(|| {
            with_cleanup(
                || {
                    std::env::set_var("MMTK_STRESS_FACTOR", "4096");

                    let options = Options::default();
                    assert_eq!(*options.stress_factor, 4096);
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
                    assert_eq!(*options.stress_factor, 4096);
                    assert!(*options.no_finalizer);
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
                    assert_eq!(*options.stress_factor, DEFAULT_STRESS_FACTOR);
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
                    assert_eq!(*options.stress_factor, DEFAULT_STRESS_FACTOR);
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
                *options.work_perf_events,
                PerfEventOptions { events: vec![] }
            );
        })
    }

    #[test]
    #[cfg(all(feature = "perf_counter", feature = "work_packet_stats"))]
    fn test_work_perf_events_option_from_env_var() {
        serial_test(|| {
            with_cleanup(
                || {
                    std::env::set_var("MMTK_WORK_PERF_EVENTS", "PERF_COUNT_HW_CPU_CYCLES,0,-1");

                    let options = Options::default();
                    assert_eq!(
                        *options.work_perf_events,
                        PerfEventOptions {
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
    #[cfg(all(feature = "perf_counter", feature = "work_packet_stats"))]
    fn test_invalid_work_perf_events_option_from_env_var() {
        serial_test(|| {
            with_cleanup(
                || {
                    // The option needs to start with "hello", otherwise it is invalid.
                    std::env::set_var("MMTK_WORK_PERF_EVENTS", "PERF_COUNT_HW_CPU_CYCLES");

                    let options = Options::default();
                    // invalid value from env var, use default.
                    assert_eq!(
                        *options.work_perf_events,
                        PerfEventOptions { events: vec![] }
                    );
                },
                || {
                    std::env::remove_var("MMTK_WORK_PERF_EVENTS");
                },
            )
        })
    }

    #[test]
    #[cfg(not(feature = "perf_counter"))]
    fn test_phase_perf_events_option_without_feature() {
        serial_test(|| {
            with_cleanup(
                || {
                    // We did not enable the perf_counter feature. The option will be invalid anyway, and will be set to empty.
                    std::env::set_var("MMTK_PHASE_PERF_EVENTS", "PERF_COUNT_HW_CPU_CYCLES,0,-1");

                    let options = Options::default();
                    // invalid value from env var, use default.
                    assert_eq!(
                        *options.work_perf_events,
                        PerfEventOptions { events: vec![] }
                    );
                },
                || {
                    std::env::remove_var("MMTK_PHASE_PERF_EVENTS");
                },
            )
        })
    }

    #[test]
    fn test_process_valid() {
        serial_test(|| {
            let mut options = Options::default();
            let success = options.set_from_command_line("no_finalizer", "true");
            assert!(success);
            assert!(*options.no_finalizer);
        })
    }

    #[test]
    fn test_process_invalid() {
        serial_test(|| {
            let mut options = Options::default();
            let default_no_finalizer = *options.no_finalizer;
            let success = options.set_from_command_line("no_finalizer", "100");
            assert!(!success);
            assert_eq!(*options.no_finalizer, default_no_finalizer);
        })
    }

    #[test]
    fn test_process_bulk_empty() {
        serial_test(|| {
            let mut options = Options::default();
            let success = options.set_bulk_from_command_line("");
            assert!(success);
        })
    }

    #[test]
    fn test_process_bulk_valid() {
        serial_test(|| {
            let mut options = Options::default();
            let success = options.set_bulk_from_command_line("no_finalizer=true stress_factor=42");
            assert!(success);
            assert!(*options.no_finalizer);
            assert_eq!(*options.stress_factor, 42);
        })
    }

    #[test]
    fn test_process_bulk_invalid() {
        serial_test(|| {
            let mut options = Options::default();
            let success = options.set_bulk_from_command_line("no_finalizer=true stress_factor=a");
            assert!(!success);
        })
    }

    #[test]
    fn test_set_typed_option_valid() {
        serial_test(|| {
            let mut options = Options::default();
            let success = options.no_finalizer.set(true);
            assert!(success);
            assert!(*options.no_finalizer);
        })
    }

    #[test]
    fn test_set_typed_option_invalid() {
        serial_test(|| {
            let mut options = Options::default();
            let threads = *options.threads;
            let success = options.threads.set(0);
            assert!(!success);
            assert_eq!(*options.threads, threads);
        })
    }
}
