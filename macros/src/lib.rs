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
mod plan_trace_object_impl;

const DEBUG_MACRO_OUTPUT: bool = false;

/// Generally a plan needs to add these attributes in order for the macro to work. The macro will
/// generate an implementation of `PlanTraceObject` for the plan. With `PlanTraceObject`, the plan use
/// `PlanProcessEdges` for GC tracing. The attributes only affects code generation in the macro, thus
/// only affects the generated `PlanTraceObject` implementation.
/// * add `#[derive(PlanTraceObject)]` to the plan struct.
/// * add `#[trace]` to each space field the plan struct has. If the policy is a copying policy,
///   it needs to further specify the copy semantic (`#[trace(CopySemantics::X)]`)
/// * add `#[fallback_trace]` to the parent plan if the plan is composed with other plans (or parent plans).
///   For example, `GenImmix` is composed with `Gen`, `Gen` is composed with `CommonPlan`, `CommonPlan` is composed
///   with `BasePlan`.
/// * add `#[post_scan]` to any space field that has some policy-specific post_scan_object(). For objects in those spaces,
///   `post_scan_object()` in the policy will be called after `VM::VMScanning::scan_object()`.
#[proc_macro_error]
#[proc_macro_derive(PlanTraceObject, attributes(trace, post_scan, fallback_trace))]
pub fn derive_plan_trace_object(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let output = if let syn::Data::Struct(syn::DataStruct {
        fields: syn::Fields::Named(ref fields),
        ..
    }) = input.data {
        let spaces = util::get_fields_with_attribute(fields, "trace");
        let post_scan_spaces = util::get_fields_with_attribute(fields, "post_scan");
        let fallback = util::get_unique_field_with_attribute(fields, "fallback_trace");

        let trace_object_function = plan_trace_object_impl::generate_trace_object(&spaces, &fallback, &ty_generics);
        let post_scan_object_function = plan_trace_object_impl::generate_post_scan_object(&post_scan_spaces, &fallback, &ty_generics);
        let may_move_objects_function = plan_trace_object_impl::generate_may_move_objects(&spaces, &fallback, &ty_generics);
        quote!{
            impl #impl_generics crate::plan::PlanTraceObject #ty_generics for #ident #ty_generics #where_clause {
                #trace_object_function

                #post_scan_object_function

                #may_move_objects_function
            }
        }
    } else {
        abort_call_site!("`#[derive(PlanTraceObject)]` only supports structs with named fields.")
    };

    // Debug the output - use the following code to debug the generated code (when cargo exapand is not working)
    if DEBUG_MACRO_OUTPUT {
        use quote::ToTokens;
        println!("{}", output.to_token_stream());
    }

    output.into()
}
