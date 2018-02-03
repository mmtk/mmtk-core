use std::str::FromStr;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use num_cpus;

pub struct UnsafeOptionsWrapper {
    inner_map: UnsafeCell<HashMap<CLIOptionKey, CLIOption>>
}

unsafe impl Sync for UnsafeOptionsWrapper {}

impl UnsafeOptionsWrapper {
    unsafe fn len(&self) -> usize {
        (&mut *self.inner_map.get()).len()
    }

    fn get(&self, name: &str) -> Option<&CLIOption> {
        let key = &CLIOptionKey::from_camelcase_str(name);
        unsafe {
            (&mut *self.inner_map.get()).get(key)
        }
    }

    pub unsafe fn process(&self, name: &str, value: &str) -> bool {
        trace!("Trying to process option pair: ({}, {})", name, value);
        let k = &CLIOptionKey::from_camelcase_str(name);
        let v = (&mut *self.inner_map.get()).get_mut(k).unwrap();
        match *v {
            IntOption(ref mut o) => {
                o.set(value)
            },
            BoolOption(ref mut o) => o.set(value)
        }
    }
}

pub trait CLIOptionTrait<T> {
    fn get(&self) -> T;
    fn set(&mut self, value_str: &str) -> bool;
}

pub struct CLIOptionType<T, F> {
    value: T,
    validator: F,
}

impl<T, F> CLIOptionType<T, F>
    where T: FromStr,
          T: Copy,
          F: Fn(&T) -> bool
{
    fn new(default_value: T, validator: F) -> Self {
        CLIOptionType {
            value: default_value,
            validator,
        }
    }
}

impl<T, F> CLIOptionTrait<T> for CLIOptionType<T, F>
    where T: FromStr,
          T: Copy,
          F: Fn(&T) -> bool
{
    fn set(&mut self, value_str: &str) -> bool {
        let value = match value_str.parse() {
            Ok(v) => v,
            Err(_) => return false
        };
        if (self.validator)(&value) {
            trace!("value: {} passed validation", value_str);
            self.value = value;
            return true;
        }
        trace!("value: {} failed validation", value_str);
        false
    }

    fn get(&self) -> T {
        self.value
    }
}

pub enum CLIOption {
    IntOption(&'static mut CLIOptionTrait<usize>),
    BoolOption(&'static mut CLIOptionTrait<bool>),
}

use self::CLIOption::*;

#[derive(Hash, Eq, PartialEq)]
pub enum CLIOptionKey {
    Threads,
    UseShortStackScans,
    UseReturnBarrier,
    EagerCompleteSweep,
}

impl CLIOptionKey {
    fn from_camelcase_str(s: &str) -> CLIOptionKey {
        match s {
            "threads" => Threads,
            "useShortStackScans" => UseShortStackScans,
            "useReturnBarrier" => UseReturnBarrier,
            "eagerCompleteSweep" => EagerCompleteSweep,
            _ => panic!("Invalid CLIOptionKey")
        }
    }
}

use self::CLIOptionKey::*;

lazy_static! {
    pub static ref OptionMap: UnsafeOptionsWrapper = UnsafeOptionsWrapper { inner_map: UnsafeCell::new({
        let mut map = HashMap::new();
        unsafe {
            map.insert(Threads, IntOption(&mut *Box::into_raw(Box::new(
                       CLIOptionType::new(num_cpus::get(), |v| *v > 0)))));
            map.insert(UseShortStackScans, BoolOption(&mut *Box::into_raw(Box::new(
                       CLIOptionType::new(false, |v| true)))));
            map.insert(UseReturnBarrier, BoolOption(&mut *Box::into_raw(Box::new(
                       CLIOptionType::new(false, |v| true)))));
            map.insert(EagerCompleteSweep, BoolOption(&mut *Box::into_raw(Box::new(
                       CLIOptionType::new(false, |v| true)))));
        }
        map
    })};
}
