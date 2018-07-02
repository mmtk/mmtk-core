lazy_static! {
    pub static ref STATS: Stats = Stats::new();
}

pub struct Stats {
    pub gc_count: usize
}

impl Stats {
    pub fn start_gc(&mut self) {
        self.gc_count += 1;
    }

    pub fn end_gc(&mut self) {
    }

    pub fn print_stats(&self) {
        println!("========================= Rust MMTk Statistics Totals =========================");
        println!("GC Count: {}", self.gc_count);
        println!("----------------------- End Rust MMTk Statistics Totals -----------------------")
    }

    pub fn new() -> Self {
        Stats {
            gc_count: 0
        }
    }
}