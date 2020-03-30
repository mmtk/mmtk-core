use crate::util::Address;
use crate::util::ObjectReference;

use crate::plan::SelectedPlan;
use crate::plan::Plan;

use std;
use crate::vm::VMBinding;

pub fn scan_region<VM: VMBinding>(plan: &SelectedPlan<VM>){
    loop {
        let mut buf = String::new();
        println!("start end <value>");
        let bytes = std::io::stdin().read_line(&mut buf).unwrap();
        let mut iter = buf.split_whitespace();
        let start = iter.next();
        let end = iter.next();
        let value = iter.next();
        if start.is_none() || bytes == 0 {
            break;
        }
        let mut start = usize::from_str_radix(&start.unwrap()[2..], 16).unwrap();
        let end = usize::from_str_radix(&end.unwrap()[2..], 16).unwrap();

        while start < end {
            let slot = unsafe {Address::from_usize(start)};
            let object: ObjectReference = unsafe {slot.load()};
            if let Some(value) = value {
                let value = usize::from_str_radix(&value[2..], 16).unwrap();
                if object.to_address() ==  unsafe {Address::from_usize(value)} {
                    println!("{} REF: {}", slot, object);
                }
            } else if plan.is_bad_ref(object) {
                println!("{} REF: {}", slot, object);
            }

            start += std::mem::size_of::<usize>();
        }
    }
}