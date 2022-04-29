extern crate proc_macro;
extern crate syn;
extern crate proc_macro_error;
extern crate quote;

use proc_macro::TokenStream;
use proc_macro_error::proc_macro_error;
use syn::{parse_macro_input};
use proc_macro_error::abort_call_site;
use quote::quote;
use syn::DeriveInput;

mod util;
mod derive_impl;

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
#[proc_macro_derive(PlanTraceObject, attributes(trace, policy_scan, fallback_trace))]
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

        let trace_object_function = derive_impl::generate_trace_object(&spaces, &fallback, &ty_generics);
        let scan_object_function = derive_impl::generate_scan_object(&scan_spaces, &ty_generics);
        let may_move_objects_function = derive_impl::generate_may_move_objects(&spaces, &fallback, &ty_generics);
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
