extern crate proc_macro;
extern crate syn;
extern crate proc_macro_error;
extern crate quote;

use proc_macro::TokenStream;
use proc_macro_error::proc_macro_error;
use syn::{parse_macro_input};
use proc_macro_error::abort_call_site;
use quote::quote;
use syn::{DeriveInput, Field, TypeGenerics};
use syn::__private::TokenStream2;

mod util;

/// Generally a plan needs to add these attributes in order for the macro to work:
/// * add `#[derive(PlanTraceObject)]` to the plan struct.
/// * add `#[trace]` to each space field the plan struct has. If the policy is a copying policy,
///   it needs to further specify the copy semantic (`#[trace(CopySemantics::X)]`)
/// * add `#[fallback_trace]` to the parent plan if the plan is composed with other plans (or parent plans).
///   For example, `GenImmix` is composed with `Gen`, `Gen` is composed with `CommonPlan`, `CommonPlan` is composed
///   with `BasePlan`.
/// * add `#[policy_scan]` to any space field that has some policy-specific scan_object(). For objects in those spaces,
///   `scan_object()` in the policy will be called. For other objects, directly call `VM::VMScanning::scan_object()`.
#[proc_macro_error]
#[proc_macro_derive(PlanTraceObject, attributes(trace, policy_scan, copy, fallback_trace))]
pub fn derive_plan_trace_object(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let output = if let syn::Data::Struct(syn::DataStruct {
        fields: syn::Fields::Named(ref fields),
        ..
    }) = input.data {
        let spaces = util::get_fields_with_attribute(fields, "trace");
        let scan_spaces = util::get_fields_with_attribute(fields, "policy_scan");
        let fallback = util::get_unique_field_with_attribute(fields, "fallback_trace");

        let trace_object_function = generate_trace_object(&spaces, &fallback, &ty_generics);
        let scan_object_function = generate_scan_object(&scan_spaces, &ty_generics);
        let may_move_objects_function = generate_may_move_objects(&spaces, &fallback, &ty_generics);
        quote!{
            impl #impl_generics crate::plan::transitive_closure::PlanTraceObject #ty_generics for #ident #ty_generics #where_clause {
                #[inline(always)]
                #trace_object_function

                #[inline(always)]
                #scan_object_function

                #[inline(always)]
                #may_move_objects_function
            }
        }
    } else {
        abort_call_site!("`#[derive(PlanTraceObject)]` only supports structs with named fields.")
    };

    // Debug the output
    // println!("{}", output.to_token_stream());

    output.into()
}

fn generate_trace_object<'a>(
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
                return <#f_ty as PolicyTraceObject #ty_generics>::trace_object::<T, KIND>(&self.#f_ident, __mmtk_trace, __mmtk_objref, #copy, __mmtk_worker);
            }
        }
    });

    // Generate a fallback to the parent plan
    let parent_field_delegator = if let Some(f) = parent_field {
        let f_ident = f.ident.as_ref().unwrap();
        let ref f_ty = f.ty;
        quote! {
            <#f_ty as PlanTraceObject #ty_generics>::trace_object::<T, KIND>(&self.#f_ident, __mmtk_trace, __mmtk_objref, __mmtk_worker)
        }
    } else {
        quote! {
            panic!("No more spaces to try")
        }
    };

    quote! {
        fn trace_object<T: crate::plan::TransitiveClosure, const KIND: crate::policy::gc_work::TraceKind>(&self, __mmtk_trace: &mut T, __mmtk_objref: crate::util::ObjectReference, __mmtk_worker: &mut crate::scheduler::GCWorker<VM>) -> crate::util::ObjectReference {
            use crate::policy::space::Space;
            use crate::policy::gc_work::PolicyTraceObject;
            use crate::plan::transitive_closure::PlanTraceObject;
            #(#space_field_handler)*
            #parent_field_delegator
        }
    }
}

fn generate_scan_object<'a>(
    scan_object_fields: &[&'a Field],
    ty_generics: &TypeGenerics,
) -> TokenStream2 {
    let scan_field_handler = scan_object_fields.iter().map(|f| {
        let f_ident = f.ident.as_ref().unwrap();
        let ref f_ty = f.ty;

        quote! {
            if self.#f_ident.in_space(__mmtk_objref) {
                use crate::policy::gc_work::PolicyTraceObject;
                <#f_ty as PolicyTraceObject #ty_generics>::scan_object::<EV>(&self.#f_ident, __mmtk_worker_tls, __mmtk_objref, __mmtk_ev);
                return;
            }
        }
    });

    quote! {
        fn scan_object<EV: crate::vm::EdgeVisitor>(&self, __mmtk_worker_tls: crate::util::opaque_pointer::VMWorkerThread, __mmtk_objref: crate::util::ObjectReference, __mmtk_ev: &mut EV) {
            use crate::vm::Scanning;

            // Plan specific
            #(#scan_field_handler)*

            // Default
            VM::VMScanning::scan_object(__mmtk_worker_tls, __mmtk_objref, __mmtk_ev);
        }
    }
}

// The generated function needs to be inlined and constant folded. Otherwise, there will be a huge
// performance penalty.
fn generate_may_move_objects<'a>(
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
        fn may_move_objects<const KIND: crate::policy::gc_work::TraceKind>() -> bool {
            use crate::policy::gc_work::PolicyTraceObject;
            use crate::plan::transitive_closure::PlanTraceObject;

            false #(#space_handlers)* #parent_handler
        }
    }
}
