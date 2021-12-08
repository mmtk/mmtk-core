use crate::vm::VMBinding;

macro_rules! define_erased_vm_mut_ref {
    ($new_type: ident = $orig_type: ty) => {
        pub struct $new_type(usize);
        impl $new_type {
            pub fn new<VM: VMBinding>(r: &mut $orig_type) -> Self {
                Self ( unsafe { std::mem::transmute(r) })
            }
            pub fn as_mut<'a, VM: VMBinding>(self) -> &'a mut $orig_type {
                unsafe { std::mem::transmute(self.0) }
            }
        }
    }
}

pub(crate) use define_erased_vm_mut_ref;
