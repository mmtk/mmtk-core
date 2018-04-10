#![warn(non_snake_case)]
#![warn(non_upper_case_globals)]
#![warn(unused)]
#![warn(unreachable_code)]

use ::{
    policy::space::{
            Space, //trait Space<PR: PageResource<Self>>: Sized + 'static
            CommonSpace //struct CommonSpace<S: Space<PR>, PR: PageResource<S>>
    },
    util::heap::PageResource //trait PageResource<S: Space<Self>>: Sized
};
use std::{
    cell::UnsafeCell
};

// FreeListSpace:
    pub trait FreeListSpace : Space {
    }

// AFreeListSpace:
    #[derive(Debug)]
    #[repr(C)]
    struct AFreeListSpace<PR: PageResource<Space = This>, This: Space<PR = PR, This = This>> {
        common: UnsafeCell<CommonSpace<PR>>,
    }

    impl<PR: PageResource<Space = This>, This: Space<PR = PR, This = This>>
            FreeListSpace for
            AFreeListSpace<PR, This> {
    }

    unsafe impl<PR: PageResource<Space = This>, This: Space<PR = PR, This = This>>
            Space for
            AFreeListSpace<PR, This> {
        type PR = PR;
        type This = This;
        fn common(&self) -> &CommonSpace<PR> {
            unsafe{&*self.common.get()}
        }

        fn common_mut(&self) -> &mut CommonSpace<PR> {
            unsafe{&mut *self.common.get()}
        }
        fn init(&mut self) { unimplemented!() }
    }

// MarkSweepSpace
    #[repr(C)]
    #[derive(Debug)]
    struct MarkSweepSpace<PR: PageResource<Space = MarkSweepSpace<PR, FL>>, FL: FreeListSpace<PR = PR, This = MarkSweepSpace<PR, FL>>> {
        base: FL,
    }

    unsafe impl<PR: PageResource<Space = MarkSweepSpace<PR, FL>>, FL: FreeListSpace<PR = PR, This = MarkSweepSpace<PR, FL>>>
            Space for
            MarkSweepSpace<PR, FL> {
        type PR = PR;
        type This = Self;
        fn init(&mut self) { unimplemented!() }
    }