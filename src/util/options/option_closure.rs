use std::str::FromStr;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use num_cpus;

use libc::c_void;

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
    validator: Option<fn(usize) -> bool>
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
    validator: Option<fn(bool) -> bool>
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
        match s {
            "threads" => self.threads.set(val),
            "useShortStackScans" => self.use_short_stack_scans.set(val),
            "useReturnBarrier" => self.use_return_barrier.set(val),
            "eagerCompleteSweep" => self.eager_complete_sweep.set(val),
            _ => panic!("Invalid CLIOptionKey")
        }
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
