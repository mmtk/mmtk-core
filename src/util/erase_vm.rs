macro_rules! define_erased_vm_mut_ref {
    ($new_type: ident = $orig_type: ty) => {
        pub struct $new_type<'a>(usize, PhantomData<&'a ()>);
        impl<'a> $new_type<'a> {
            #[inline(always)]
            pub fn new<VM: VMBinding>(r: &'a mut $orig_type) -> Self {
                Self ( unsafe { std::mem::transmute(r) }, PhantomData)
            }
            #[inline(always)]
            pub fn as_mut<VM: VMBinding>(self) -> &'a mut $orig_type {
                unsafe { std::mem::transmute(self.0) }
            }
        }
    }
}

pub(crate) use define_erased_vm_mut_ref;
