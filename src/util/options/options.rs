use std::str::FromStr;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use num_cpus;

use libc::c_void;

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

pub struct UnsafeOptionsWrapper {
    inner_map: UnsafeCell<Options>
}

unsafe impl Sync for UnsafeOptionsWrapper {}

impl UnsafeOptionsWrapper {
    pub fn get(&self) -> &Options {
        unsafe {
            (&*self.inner_map.get())
        }
    }

    pub unsafe fn process(&self, name: &str, value: &str) -> bool {
        (&mut *self.inner_map.get()).set_from_camelcase_str(name, value)
    }
}

pub trait CLIOption<T: Default> {
    fn new(default: T, validator: Option<fn(T) -> bool>) -> Self;
    fn get(&self) -> T;
    fn set(&mut self, value: &str) -> bool;
}

pub struct IntOption {
    value: usize,
    validator: Option<fn(usize) -> bool>,
}

impl CLIOption<usize> for IntOption {
    fn new(default: usize, validator: Option<fn(usize) -> bool>) -> Self {
        Self {
            value: default,
            validator,
        }
    }

    fn get(&self) -> usize {
        self.value
    }

    fn set(&mut self, value: &str) -> bool {
        if let Ok(dval) = value.parse() {
            let succ = self.validator.unwrap_or(|_| true)(dval);
            if succ { self.value = dval; }
            succ
        } else {
            false
        }
    }
}

pub struct BoolOption {
    value: bool,
    validator: Option<fn(bool) -> bool>,
}

impl CLIOption<bool> for BoolOption {
    fn new(default: bool, validator: Option<fn(bool) -> bool>) -> Self {
        Self {
            value: default,
            validator,
        }
    }

    fn get(&self) -> bool {
        self.value
    }

    fn set(&mut self, value: &str) -> bool {
        if let Ok(dval) = value.parse() {
            let succ = self.validator.unwrap_or(|_| true)(dval);
            if succ { self.value = dval; }
            succ
        } else {
            false
        }
    }
}

pub struct Options {
    pub threads: IntOption,
    pub use_short_stack_scans: BoolOption,
    pub use_return_barrier: BoolOption,
    pub eager_complete_sweep: BoolOption,
}

impl Options {
    fn set_from_camelcase_str(&mut self, s: &str, val: &str) -> bool {
        trace!("Trying to process option pair: ({}, {})", s, val);
        let result = match s {
            "threads" => self.threads.set(val),
            "useShortStackScans" => self.use_short_stack_scans.set(val),
            "useReturnBarrier" => self.use_return_barrier.set(val),
            "eagerCompleteSweep" => self.eager_complete_sweep.set(val),
            _ => panic!("Invalid Options key")
        };
        if result {
            trace!("Validation passed");
        } else {
            trace!("Validation failed")
        }
        result
    }
}

lazy_static! {
    pub static ref OptionMap: UnsafeOptionsWrapper = UnsafeOptionsWrapper { inner_map: UnsafeCell::new(
        Options {
            threads: IntOption::new(num_cpus::get(), Some(|v| v > 0)),
            use_short_stack_scans: BoolOption::new(false, None),
            use_return_barrier: BoolOption::new(false, None),
            eager_complete_sweep: BoolOption::new(false, None),
        })};
}
