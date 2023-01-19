use quote::quote;
use syn::{Field, TypeGenerics};
use proc_macro2::TokenStream as TokenStream2;

use crate::util;

pub(crate) fn generate_trace_object<'a>(
    space_fields: &[&'a Field],
    parent_field: &Option<&'a Field>,
    ty_generics: &TypeGenerics,
) -> TokenStream2 {
    // Generate a check with early return for each space
    let space_field_handler = space_fields.iter().map(|f| {
        let f_ident = f.ident.as_ref().unwrap();
        let ref f_ty = f.ty;

        // Figure out copy
        let trace_attr = util::get_field_attribute(f, "trace").unwrap();
        let copy = if !trace_attr.tokens.is_empty() {
            use syn::Token;
            use syn::NestedMeta;
            use syn::punctuated::Punctuated;

            let args = trace_attr.parse_args_with(Punctuated::<NestedMeta, Token![,]>::parse_terminated).unwrap();
            // CopySemantics::X is a path.
            if let Some(NestedMeta::Meta(syn::Meta::Path(p))) = args.first() {
                quote!{ Some(#p) }
            } else {
                quote!{ None }
            }
        } else {
            quote!{ None }
        };

        quote! {
            if self.#f_ident.in_space(__mmtk_objref) {
                return <#f_ty as PolicyTraceObject #ty_generics>::trace_object::<Q, KIND>(&self.#f_ident, __mmtk_queue, __mmtk_objref, #copy, __mmtk_worker);
            }
        }
    });

    // Generate a fallback to the parent plan
    let parent_field_delegator = if let Some(f) = parent_field {
        let f_ident = f.ident.as_ref().unwrap();
        let ref f_ty = f.ty;
        quote! {
            <#f_ty as PlanTraceObject #ty_generics>::trace_object::<Q, KIND>(&self.#f_ident, __mmtk_queue, __mmtk_objref, __mmtk_worker)
        }
    } else {
        quote! {
            <VM::VMActivePlan as crate::vm::ActivePlan<VM>>::vm_trace_object::<Q>(__mmtk_queue, __mmtk_objref, __mmtk_worker)
        }
    };

    quote! {
        #[inline(always)]
        fn trace_object<Q: crate::plan::ObjectQueue, const KIND: crate::policy::gc_work::TraceKind>(&self, __mmtk_queue: &mut Q, __mmtk_objref: crate::util::ObjectReference, __mmtk_worker: &mut crate::scheduler::GCWorker<VM>) -> crate::util::ObjectReference {
            use crate::policy::space::Space;
            use crate::policy::gc_work::PolicyTraceObject;
            use crate::plan::PlanTraceObject;
            #(#space_field_handler)*
            #parent_field_delegator
        }
    }
}

pub(crate) fn generate_post_scan_object<'a>(
    post_scan_object_fields: &[&'a Field],
    parent_field: &Option<&'a Field>,
    ty_generics: &TypeGenerics,
) -> TokenStream2 {
    let scan_field_handler = post_scan_object_fields.iter().map(|f| {
        let f_ident = f.ident.as_ref().unwrap();
        let ref f_ty = f.ty;

        quote! {
            if self.#f_ident.in_space(__mmtk_objref) {
                use crate::policy::gc_work::PolicyTraceObject;
                <#f_ty as PolicyTraceObject #ty_generics>::post_scan_object(&self.#f_ident, __mmtk_objref);
                return;
            }
        }
    });

    // Generate a fallback to the parent plan
    let parent_field_delegator = if let Some(f) = parent_field {
        let f_ident = f.ident.as_ref().unwrap();
        let ref f_ty = f.ty;
        quote! {
            <#f_ty as PlanTraceObject #ty_generics>::post_scan_object(&self.#f_ident, __mmtk_objref)
        }
    } else {
        TokenStream2::new()
    };

    quote! {
        #[inline(always)]
        fn post_scan_object(&self, __mmtk_objref: crate::util::ObjectReference) {
            use crate::plan::PlanTraceObject;
            #(#scan_field_handler)*
            #parent_field_delegator
        }
    }
}

// The generated function needs to be inlined and constant folded. Otherwise, there will be a huge
// performance penalty.
pub(crate) fn generate_may_move_objects<'a>(
    space_fields: &[&'a Field],
    parent_field: &Option<&'a Field>,
    ty_generics: &TypeGenerics,
) -> TokenStream2 {
    // If any space or the parent may move objects, the plan may move objects
    let space_handlers = space_fields.iter().map(|f| {
        let ref f_ty = f.ty;

        quote! {
            || <#f_ty as PolicyTraceObject #ty_generics>::may_move_objects::<KIND>()
        }
    });

    let parent_handler = if let Some(p) = parent_field {
        let ref p_ty = p.ty;

        quote! {
            || <#p_ty as PlanTraceObject #ty_generics>::may_move_objects::<KIND>()
        }
    } else {
        TokenStream2::new()
    };

    quote! {
        #[inline(always)]
        fn may_move_objects<const KIND: crate::policy::gc_work::TraceKind>() -> bool {
            use crate::policy::gc_work::PolicyTraceObject;
            use crate::plan::PlanTraceObject;

            false #(#space_handlers)* #parent_handler
        }
    }
}
