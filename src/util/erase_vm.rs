use crate::vm::VMBinding;
use std::marker::PhantomData;

macro_rules! define_erased_vm_ref {
    ($new_type: ident = $orig_type: ty) => {
        #[derive(Copy, Clone)]
        pub struct $new_type<'a>(usize, PhantomData<&'a ()>);
        impl<'a> $new_type<'a> {
            pub fn new<VM: VMBinding>(r: &'a $orig_type) -> Self {
                Self ( unsafe { std::mem::transmute(r) }, PhantomData)
            }
            #[inline(always)]
            pub fn as_ref<VM: VMBinding>(self) -> &'a $orig_type {
                unsafe { std::mem::transmute(self.0) }
            }
        }
    }
}

pub(crate) use define_erased_vm_ref;