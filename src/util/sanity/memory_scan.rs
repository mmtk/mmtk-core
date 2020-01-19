use util::Address;
use util::ObjectReference;

use ::plan::SelectedPlan;
use ::plan::Plan;

use std;

pub fn scan_region(plan: &SelectedPlan){
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
            if value.is_none() {
                if plan.is_bad_ref(object) {
                    println!("{} REF: {}", slot, object);
                }
            } else {
                let value = usize::from_str_radix(&value.unwrap()[2..], 16).unwrap();
                if object.to_address() ==  unsafe {Address::from_usize(value)} {
                    println!("{} REF: {}", slot, object);
                }
            }

            start += std::mem::size_of::<usize>();
        }
    }
}