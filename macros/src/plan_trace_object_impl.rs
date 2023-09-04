use proc_macro2::TokenStream as TokenStream2;
use proc_macro_error::abort_call_site;
use quote::quote;
use syn::{DeriveInput, Expr, Field, TypeGenerics};

use crate::util;

pub(crate) fn derive(input: DeriveInput) -> TokenStream2 {
    let ident = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let syn::Data::Struct(syn::DataStruct {
        fields: syn::Fields::Named(ref fields),
        ..
    }) = input.data
    else {
        abort_call_site!("`#[derive(PlanTraceObject)]` only supports structs with named fields.");
    };

    let spaces = util::get_fields_with_attribute(fields, "space");
    let post_scan_spaces = util::get_fields_with_attribute(fields, "post_scan");
    let parent = util::get_unique_field_with_attribute(fields, "parent");

    let trace_object_function = generate_trace_object(&spaces, &parent, &ty_generics);
    let post_scan_object_function =
        generate_post_scan_object(&post_scan_spaces, &parent, &ty_generics);
    let may_move_objects_function = generate_may_move_objects(&spaces, &parent, &ty_generics);

    quote! {
        impl #impl_generics crate::plan::PlanTraceObject #ty_generics for #ident #ty_generics #where_clause {
            #trace_object_function

            #post_scan_object_function

            #may_move_objects_function
        }
    }
}

pub(crate) fn generate_trace_object<'a>(
    space_fields: &[&'a Field],
    parent_field: &Option<&'a Field>,
    ty_generics: &TypeGenerics,
) -> TokenStream2 {
    // Generate a check with early return for each space
    let space_field_handler = space_fields.iter().map(|f| {
        let f_ident = f.ident.as_ref().unwrap();
        let f_ty = &f.ty;

        // Figure out copy
        let maybe_copy_semantics_attr = util::get_field_attribute(f, "copy_semantics");
        let copy = match maybe_copy_semantics_attr {
            None => quote!{ None },
            Some(attr) => match &attr.meta {
                syn::Meta::Path(_) => {
                    // #[copy_semantics]
                    abort_call_site!("The `#[copy_semantics(expr)]` macro needs an argument.");
                },
                syn::Meta::List(list) => {
                    // #[copy_semantics(BlahBlah)]
                    let copy_semantics = list.parse_args::<Expr>().unwrap_or_else(|_| {
                        abort_call_site!("In `#[copy_semantics(expr)]`, expr must be an expression.");
                    });
                    quote!{ Some(#copy_semantics) }
                },
                syn::Meta::NameValue(_) => {
                    // #[copy_semantics = BlahBlah]
                    abort_call_site!("The #[copy_semantics] macro does not support the name-value form.");
                },
            }
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
        let f_ty = &f.ty;
        quote! {
            <#f_ty as PlanTraceObject #ty_generics>::trace_object::<Q, KIND>(&self.#f_ident, __mmtk_queue, __mmtk_objref, __mmtk_worker)
        }
    } else {
        quote! {
            <VM::VMActivePlan as crate::vm::ActivePlan<VM>>::vm_trace_object::<Q>(__mmtk_queue, __mmtk_objref, __mmtk_worker)
        }
    };

    quote! {
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
        let f_ty = &f.ty;

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
        let f_ty = &f.ty;
        quote! {
            <#f_ty as PlanTraceObject #ty_generics>::post_scan_object(&self.#f_ident, __mmtk_objref)
        }
    } else {
        TokenStream2::new()
    };

    quote! {
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
        let f_ty = &f.ty;

        quote! {
            || <#f_ty as PolicyTraceObject #ty_generics>::may_move_objects::<KIND>()
        }
    });

    let parent_handler = if let Some(p) = parent_field {
        let p_ty = &p.ty;

        quote! {
            || <#p_ty as PlanTraceObject #ty_generics>::may_move_objects::<KIND>()
        }
    } else {
        TokenStream2::new()
    };

    quote! {
        fn may_move_objects<const KIND: crate::policy::gc_work::TraceKind>() -> bool {
            use crate::policy::gc_work::PolicyTraceObject;
            use crate::plan::PlanTraceObject;

            false #(#space_handlers)* #parent_handler
        }
    }
}
