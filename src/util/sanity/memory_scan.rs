use crate::util::Address;
use crate::util::ObjectReference;

// This is legacy code, and no one is using this. Using gdb can achieve the same thing for debugging.
// The JikesRVM binding still declares this method and we need to remove it from JikesRVM.
#[deprecated]
pub fn scan_region() {
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
            let slot = unsafe { Address::from_usize(start) };
            let object: ObjectReference = unsafe { slot.load() };
            if let Some(value) = value {
                let value = usize::from_str_radix(&value[2..], 16).unwrap();
                if object.to_address() == unsafe { Address::from_usize(value) } {
                    println!("{} REF: {}", slot, object);
                }
            } else if !object.is_sane() {
                println!("{} REF: {}", slot, object);
            }
            // FIXME steveb Consider VM-specific integrity check on reference.
            start += std::mem::size_of::<usize>();
        }
    }
}
