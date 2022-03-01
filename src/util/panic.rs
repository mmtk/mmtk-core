fn mmtk_panic() {
    use crate::mmtk::SFT_MAP;

    println!("===== Internal Error in MMTk =====");
    println!("Something went wrong with MMTk.");
    println!();

    println!("Dumping space function table (SFT)...");
    println!("{}", SFT_MAP.print_sft_map());
}

pub(crate) fn set_mmtk_panic_hook() {
    use std::panic;

    let default_handler = panic::take_hook();

    panic::set_hook(Box::new(move |info| {
        mmtk_panic();
        default_handler(info)
    }));
}
