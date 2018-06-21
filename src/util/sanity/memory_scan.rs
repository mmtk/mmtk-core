use util::Address;
use util::ObjectReference;

use ::plan::selected_plan::PLAN;
use ::plan::Plan;

use std;

pub fn scan_region(){
    let mut temp = String::new();
    loop {
        std::io::stdin().read_line(&mut temp).unwrap();
        let mut iter = temp.split_whitespace();
        let start = iter.next();
        let end = iter.next();
        if start.is_none() {
            break;
        }
        let mut start = usize::from_str_radix(&start.unwrap()[2..], 16).unwrap();
        let end = usize::from_str_radix(&end.unwrap()[2..], 16).unwrap();

        while start < end {
            let slot = unsafe {Address::from_usize(start)};
            let object: ObjectReference = unsafe {slot.load()};
            if PLAN.is_bad_ref(object) {
                println!("{} REF: {}", slot, object);
            }

            start += std::mem::size_of::<usize>();
        }
    }
}