use ::util::address::Address;
use ::policy::space::Space;

pub trait PageResource<S: Space<Self>>: Sized {
    /// Allocate pages from this resource.
    /// Simply bump the cursor, and fail if we hit the sentinel.
    /// Return The start of the first page if successful, zero on failure.
    fn get_new_pages(&self, reserved_pages: usize, required_pages: usize, zeroed: bool) -> Address {
        self.alloc_pages(reserved_pages, required_pages, zeroed)
    }

    fn reserve_pages(&self, pages: usize) -> usize;

    fn clear_request(&self, reserved_pages: usize);

    fn update_zeroing_approach(&self, nontemporal: bool, concurrent: bool);

    fn skip_concurrent_zeroing(&self);

    fn trigger_concurrent_zeroing(&self);

    fn concurrent_zeroing(&self);

    fn alloc_pages(&self, reserved_pages: usize, required_pages: usize, zeroed: bool) -> Address;

    fn adjust_for_metadata(&self, pages: usize);

    fn commit_pages(&self, reserved_pages: usize, actual_pages: usize);

    fn reserved_pages(&self) -> usize;

    fn committed_pages(&self) -> usize;

    fn cumulative_committed_pages() -> usize;


    fn bind_space(&mut self, space: &'static S);
}