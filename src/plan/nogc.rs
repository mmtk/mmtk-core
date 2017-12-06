use std::sync::Mutex;
use ::policy::space::Space;
use ::util::alloc::bumpallocator::BumpAllocator;

lazy_static! {
    pub static ref SPACE: Mutex<Space> = Mutex::new(Space::new());
}

pub type SelectedAllocator<'a> = BumpAllocator<'a>;