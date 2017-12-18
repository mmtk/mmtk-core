use ::util::address::Address;

pub trait PageResource {
    fn new() -> Self;
    /// Contiguous monotone resource. The address range is pre-defined at
    /// initialization time and is immutable.
    fn init(&mut self, heap_size: usize);
    /// Allocate pages from this resource.
    /// Simply bump the cursor, and fail if we hit the sentinel.
    /// Return The start of the first page if successful, zero on failure.
    fn get_new_pages(&mut self, size: usize) -> Address;
    /// The start of this region of memory
    fn get_start(&self) -> Address;
    /// The size of the region of memory
    fn get_extend(&self) -> usize;
}