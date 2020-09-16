use crate::util::constants::LOG_BYTES_IN_PAGE;
use std::cell::UnsafeCell;
use std::default::Default;
use std::ops::Deref;

custom_derive! {
    #[derive(Copy, Clone, EnumFromStr)]
    pub enum NurseryZeroingOptions {
        Temporal,
        Nontemporal,
        Concurrent,
        Adaptive,
    }
}

pub struct UnsafeOptionsWrapper(UnsafeCell<Options>);
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

fn always_valid<T>(_: T) -> bool {
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
                    $(stringify!($name) => if let Ok(val) = val.parse() {
                        self.$name = val;
                        let validate_fn = $validator;
                        validate_fn(val)
                    } else {
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

                // If we have env vars that start with MMTK_ and matches any option (such as MMTK_STRESS_FACTOR),
                // we set the option to its value (if it is a valid value). Otherwise, use the defualt value.
                const PREFIX: &str = "MMTK_";
                for (key, val) in std::env::vars() {
                    if key.starts_with(PREFIX) {
                        // strip the prefix, and get the lower case string
                        let rest_of_key: &str = &key[PREFIX.len()..].to_lowercase();
                        match rest_of_key {
                            $(stringify!($name) => { options.set_from_str(rest_of_key, &val); },)*
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
    threads:               usize                [|v| v > 0]    = num_cpus::get(),
    use_short_stack_scans: bool                 [always_valid] = false,
    use_return_barrier:    bool                 [always_valid] = false,
    eager_complete_sweep:  bool                 [always_valid] = false,
    ignore_system_g_c:     bool                 [always_valid] = false,
    // Note: Not used. To workaround cmd args passed by the running script
    variable_size_heap:    bool                 [always_valid] = true,
    no_finalizer:          bool                 [always_valid] = false,
    no_reference_types:    bool                 [always_valid] = false,
    nursery_zeroing:       NurseryZeroingOptions[always_valid] = NurseryZeroingOptions::Temporal,
    // Note: This gets ignored. Use RUST_LOG to specify log level.
    // TODO: Delete this option.
    verbose:               usize                [always_valid] = 0,
    stress_factor:         usize                [always_valid] = usize::max_value() >> LOG_BYTES_IN_PAGE,
    // vmspace
    // FIXME: These options are set for JikesRVM. We need a proper way to set options.
    //   We need to set these values programmatically in VM specific code.
    vm_space:              bool                 [always_valid] = true,
    vm_space_size:         usize                [|v| v > 0]    = 0x7cc_cccc,
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
    use crate::util::constants::LOG_BYTES_IN_PAGE;
    use crate::util::options::Options;
    use crate::util::test_util::serial_test;

    const DEFAULT_STRESS_FACTOR: usize = usize::max_value() >> LOG_BYTES_IN_PAGE;

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
            std::env::set_var("MMTK_STRESS_FACTOR", "4096");

            let res = std::panic::catch_unwind(|| {
                let options = Options::default();
                assert_eq!(options.stress_factor, 4096);
            });
            assert!(res.is_ok());

            std::env::remove_var("MMTK_STRESS_FACTOR");
        })
    }

    #[test]
    fn with_multiple_valid_env_vars() {
        serial_test(|| {
            std::env::set_var("MMTK_STRESS_FACTOR", "4096");
            std::env::set_var("MMTK_VM_SPACE", "false");

            let res = std::panic::catch_unwind(|| {
                let options = Options::default();
                assert_eq!(options.stress_factor, 4096);
                assert_eq!(options.vm_space, false);
            });
            assert!(res.is_ok());

            std::env::remove_var("MMTK_STRESS_FACTOR");
            std::env::remove_var("MMTK_VM_SPACE");
        })
    }

    #[test]
    fn with_invalid_env_var_value() {
        serial_test(|| {
            // invalid value, we cannot parse the value, so use the default value
            std::env::set_var("MMTK_STRESS_FACTOR", "abc");

            let res = std::panic::catch_unwind(|| {
                let options = Options::default();
                assert_eq!(options.stress_factor, DEFAULT_STRESS_FACTOR);
            });
            assert!(res.is_ok());

            std::env::remove_var("MMTK_STRESS_FACTOR");
        })
    }

    #[test]
    fn with_invalid_env_var_key() {
        serial_test(|| {
            // invalid value, we cannot parse the value, so use the default value
            std::env::set_var("MMTK_ABC", "42");

            let res = std::panic::catch_unwind(|| {
                let options = Options::default();
                assert_eq!(options.stress_factor, DEFAULT_STRESS_FACTOR);
            });
            assert!(res.is_ok());

            std::env::remove_var("MMTK_ABC");
        })
    }
}
