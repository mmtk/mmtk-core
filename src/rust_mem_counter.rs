use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::AtomicUsize,
};

use atomic::Ordering;

use crate::util::constants::LOG_BYTES_IN_PAGE;

struct SystemAllocatorWithCounter {
    live_size: AtomicUsize,
    max_live_size: AtomicUsize,
}

unsafe impl GlobalAlloc for SystemAllocatorWithCounter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        assert!(cfg!(feature = "rust_mem_counter"));
        let size = layout.size();
        let current_size = self.live_size.fetch_add(size, Ordering::SeqCst) + size;
        self.max_live_size.fetch_max(current_size, Ordering::SeqCst);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        assert!(cfg!(feature = "rust_mem_counter"));
        self.live_size.fetch_sub(layout.size(), Ordering::SeqCst);
        System.dealloc(ptr, layout)
    }
}

#[cfg_attr(feature = "rust_mem_counter", global_allocator)]
static GLOBAL: SystemAllocatorWithCounter = SystemAllocatorWithCounter {
    live_size: AtomicUsize::new(0),
    max_live_size: AtomicUsize::new(0),
};

static MMAP_SIZE: AtomicUsize = AtomicUsize::new(0);

static PEAK_MMAP_SIZE: AtomicUsize = AtomicUsize::new(0);

static RSS: AtomicUsize = AtomicUsize::new(0);
static PEAK_RSS: AtomicUsize = AtomicUsize::new(0);
static VIRT: AtomicUsize = AtomicUsize::new(0);
static PEAK_VIRT: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct BufferSizeCounter {
    name: &'static str,
    entry_size: usize,
    entries: AtomicUsize,
    max_entries: AtomicUsize,
}

impl BufferSizeCounter {
    const fn new(name: &'static str, entry_size: usize) -> Self {
        Self {
            name,
            entry_size,
            entries: AtomicUsize::new(0),
            max_entries: AtomicUsize::new(0),
        }
    }

    pub fn add(&self, entries: usize) {
        let entries = entries + self.entries.fetch_add(entries, Ordering::SeqCst);
        self.max_entries.fetch_max(entries, Ordering::SeqCst);
    }

    pub fn sub(&self, entries: usize) {
        self.entries.fetch_sub(entries, Ordering::SeqCst);
    }

    fn report(&self) {
        gc_log!(
            " - {}: {}M peak={}M",
            self.name,
            self.entries.load(Ordering::SeqCst) * self.entry_size >> 20,
            self.max_entries.load(Ordering::SeqCst) * self.entry_size >> 20,
        );
    }
}

pub(crate) static INC_BUFFER_COUNTER: BufferSizeCounter =
    BufferSizeCounter::new("inc buffer size", 8);
pub(crate) static DEC_BUFFER_COUNTER: BufferSizeCounter =
    BufferSizeCounter::new("dec buffer size", 8);
pub(crate) static SATB_BUFFER_COUNTER: BufferSizeCounter =
    BufferSizeCounter::new("satb buffer size", 8);
pub(crate) static MATURE_EVAC_REMSET_COUNTER: BufferSizeCounter =
    BufferSizeCounter::new("mature evac remset buffer size", 16);
pub(crate) static BLOCK_ALLOC_BUFFER_COUNTER: BufferSizeCounter =
    BufferSizeCounter::new("block alloc buffer size", 8);

pub fn dump(_gc_start: bool) {
    if cfg!(feature = "rust_mem_counter") {
        update_rss();
        gc_log!(
            " - rust heap: {}M, peak = {}M",
            GLOBAL.live_size.load(Ordering::SeqCst) >> 20,
            GLOBAL.max_live_size.load(Ordering::SeqCst) >> 20,
        );
        gc_log!(
            " - mmap: {}M, peak = {}M",
            MMAP_SIZE.load(Ordering::SeqCst) >> 20,
            PEAK_MMAP_SIZE.load(Ordering::SeqCst) >> 20,
        );
        if RSS.load(Ordering::SeqCst) != 0 {
            gc_log!(
                " - VmRss: {}M, peak = {}M",
                RSS.load(Ordering::SeqCst) >> 20,
                PEAK_RSS.load(Ordering::SeqCst) >> 20,
            );
            gc_log!(
                " - VmSize: {}M, peak = {}M",
                VIRT.load(Ordering::SeqCst) >> 20,
                PEAK_VIRT.load(Ordering::SeqCst) >> 20,
            );
        }
        INC_BUFFER_COUNTER.report();
        DEC_BUFFER_COUNTER.report();
        SATB_BUFFER_COUNTER.report();
        MATURE_EVAC_REMSET_COUNTER.report();
        BLOCK_ALLOC_BUFFER_COUNTER.report();
    }
}

pub fn record_munmap(bytes: usize) {
    if cfg!(feature = "rust_mem_counter") {
        MMAP_SIZE.fetch_sub(bytes, Ordering::SeqCst);
    }
}

pub fn update_rss() {
    if cfg!(feature = "rust_mem_counter") {
        match std::fs::read_to_string("/proc/self/statm") {
            Ok(statm) => {
                let mut values = statm.trim().split_ascii_whitespace();
                let virt = values.next().unwrap().parse::<usize>().unwrap();
                let rss = values.next().unwrap().parse::<usize>().unwrap();
                VIRT.store(virt << LOG_BYTES_IN_PAGE, Ordering::SeqCst);
                PEAK_VIRT.fetch_max(virt << LOG_BYTES_IN_PAGE, Ordering::SeqCst);
                RSS.store(rss << LOG_BYTES_IN_PAGE, Ordering::SeqCst);
                PEAK_RSS.fetch_max(rss << LOG_BYTES_IN_PAGE, Ordering::SeqCst);
            }
            _ => {}
        }
    }
}
