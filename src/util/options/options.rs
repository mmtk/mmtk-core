use ::util::Address;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::mem::discriminant;

use self::CLIOptionType::*;

extern crate num_cpus;

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

lazy_static! {
    pub static ref OptionMap: UnsafeOptionsWrapper = UnsafeOptionsWrapper { inner_map: UnsafeCell::new(HashMap::new()) };
}

pub enum CLIOptionType {
    IntOption(usize),
    BoolOption(bool)
}

pub struct UnsafeOptionsWrapper {
    inner_map: UnsafeCell<HashMap<String,CLIOptionType>>
}

unsafe impl Sync for UnsafeOptionsWrapper {}

impl UnsafeOptionsWrapper {

    pub unsafe fn register(&self) {
        self.push("threads", IntOption(num_cpus::get()));
        self.push("useShortStackScans", BoolOption(false));
        self.push("useReturnBarrier", BoolOption(false));
        self.push("eagerCompleteSweep", BoolOption(false));
    }

    unsafe fn push(&self, name: &str, value: CLIOptionType){
        (&mut *self.inner_map.get()).insert(String::from(name), value);
    }

    unsafe fn len(&self) -> usize{
        (&mut *self.inner_map.get()).len()
    }

    fn get(&self, name: &str) -> Option<&CLIOptionType> {
        unsafe {
            (&mut *self.inner_map.get()).get(&String::from(name))
        }
    }

    unsafe fn validate(name: &str, value: &CLIOptionType) -> bool {
        match name {
            "threads" => {
                if let &IntOption(v) = value {
                    return v > 0
                }
            }
            _ =>  {
                return true
            }
        }
        false
    }

    pub unsafe fn process(&self, name: &str, value: &str) -> bool {
        let option = self.get(name);
        if let Some(o) = option {
            match o {
                &BoolOption(b) => {
                    match value {
                        "true" => {
                            self.push(name, BoolOption(true));
                            return true
                        }
                        "false" => {
                            self.push(name, BoolOption(false));
                            return true
                        }
                        _ => return false
                    }
                }
                &IntOption(i) => {
                    match value.parse() {
                        Ok(v) => {
                            let new_value = IntOption(v);
                            if UnsafeOptionsWrapper::validate(name,&new_value) {
                                self.push(name, new_value);
                                return true
                            }
                            return false
                        }
                        Err(e) => {
                            return false
                        }
                    }
                }
                _ => return false
            }
        } else {
            return false
        }

    }

}
