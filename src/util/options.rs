use libc::c_void;
use num_cpus;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;
use util::constants::LOG_BYTES_IN_PAGE;

/*
// Boolean Options
pub static mut ProtectOnRelease : bool = false;
pub static mut EagerCompleteSweep : bool = false;
pub static mut print_phase_stats : bool = false;
pub static mut xml_stats : bool = false;
pub static mut verbose_timing : bool = false;
pub static mut no_finalizer : bool = false;
pub static mut no_reference_types : bool = false;
pub static mut full_heap_system_gc : bool = false;
pub static mut ignore_system_gc : bool = false;
pub static mut variable_size_heap : bool = true;
pub static mut eager_mmap_spaces : bool = false;
pub static mut use_return_barrier : bool = false;
pub static mut use_short_stack_scans : bool = false;

// Int Options
pub static mut verbose : usize = 0;
pub static mut mark_sweep_mark_bits : usize = 4;
pub static mut threads : usize = 1;

// Byte Options
pub static mut stress_factor : usize = 2147479552;
pub static mut meta_data_limit : usize = 16777216;
pub static mut bounded_nursery : usize = 33554432;
pub static mut fixed_nursery : usize = 2097152;
pub static mut cycle_trigger_threshold : usize = 4194304;

// Address Options
pub static mut debug_address : Address = Address::zero();

// Float Options
pub static mut pretenure_threshold_fraction : f32 = 0.5;

// String Options
pub static mut perf_events : &str = "";

// Enum Options
pub static mut NurseryZeroing : NurseryZeroingOptions = temporal;

enum NurseryZeroingOptions {
    Temporal,
    Nontemporal,
    Concurrent,
    Adaptive
}
*/

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
    pub unsafe fn process(&self, name: &str, value: &str) -> bool {
        (&mut *self.0.get()).set_from_camelcase_str(name, value)
    }
}
impl Deref for UnsafeOptionsWrapper {
    type Target = Options;
    fn deref(&self) -> &Options {
        unsafe { (&*self.0.get()) }
    }
}

fn always_valid<T>(val: T) -> bool {
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
        fn set_from_str(o: &mut Options, s: &str, val: &str)->bool {
            match s {
                $(stringify!($name) => if let Ok(val) = val.parse() {
                    o.$name = val;
                    ($validator)(val)
                } else {
                    false
                })*
                _ => panic!("Invalid Options key")
            }
        }
        lazy_static!{
            pub static ref OPTION_MAP: UnsafeOptionsWrapper = UnsafeOptionsWrapper::new(
                Options {
                    $($name: $default),*
                }
            );
        }
    ]
}
options!{
    threads:               usize                [|v| v > 0]    = num_cpus::get(),
    use_short_stack_scans: bool                 [always_valid] = false,
    use_return_barrier:    bool                 [always_valid] = false,
    eager_complete_sweep:  bool                 [always_valid] = false,
    nursery_zeroing:       NurseryZeroingOptions[always_valid] = NurseryZeroingOptions::Temporal,
    verbose:               usize                [always_valid] = 0,
    stress_factor:         usize                [always_valid] = usize::max_value() >> LOG_BYTES_IN_PAGE,
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

        let result = set_from_str(self, sr.as_str(), val);

        if result {
            trace!("Validation passed");
        } else {
            trace!("Validation failed")
        }
        result
    }
}
