//! MMTk uses [`crate::vm::VMBinding`], which allows us to call into bindings
//! with little overhead. As a result, some types in MMTk are generic types with a type parameter `<VM>`.
//! However, in some cases, using generic types is not allowed. For example, in an object-safe trait,
//! the methods cannot be generic, thus the method's parameters cannot be generic types.
//!
//! This module defines macros that can be used to create a special ref type that erases the <VM> type parameter.
//! For example, we create a type `TErasedRef` for `&T<VM>`. `TErasedRef` has no type parameter, and
//! can be used in places where a type parameter is undesired. The type `TErasedRef` can be cast back to `&T<VM>`
//! when we supply a type parameter `<VM>`. This works under the assumption that
//! one MMTk process should only have one VM type. In such a case, when we cast from a `&T<VM>` to `TErasedRef`, and
//! cast back to `&T<VM>`, the type parameter is guaranteed to be the same. Thus the casting is correct.
//!
//! `TErasedRef` has the same lifetime as `&T<VM>`.

macro_rules! define_erased_vm_mut_ref {
    ($new_type: ident = $orig_type: ty) => {
        pub struct $new_type<'a>(usize, PhantomData<&'a ()>);
        impl<'a> $new_type<'a> {
            #[inline(always)]
            pub fn new<VM: VMBinding>(r: &'a mut $orig_type) -> Self {
                Self(unsafe { std::mem::transmute(r) }, PhantomData)
            }
            #[inline(always)]
            pub fn into_mut<VM: VMBinding>(self) -> &'a mut $orig_type {
                unsafe { std::mem::transmute(self.0) }
            }
        }
    };
}

pub(crate) use define_erased_vm_mut_ref;
