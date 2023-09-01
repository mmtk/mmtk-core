extern crate proc_macro;
extern crate proc_macro_error;
extern crate quote;
extern crate syn;

use proc_macro::TokenStream;
use proc_macro_error::abort_call_site;
use proc_macro_error::proc_macro_error;
use quote::quote;
use syn::parse_macro_input;
use syn::DeriveInput;

mod has_spaces_impl;
mod plan_trace_object_impl;
mod util;

const DEBUG_MACRO_OUTPUT: bool = false;

#[proc_macro_error]
#[proc_macro_derive(HasSpaces, attributes(space, parent))]
pub fn derive_has_spaces(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let output = has_spaces_impl::derive(input);

    output.into()
}

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
#[proc_macro_derive(PlanTraceObject, attributes(space, parent, copy_semantics, post_scan))]
pub fn derive_plan_trace_object(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let syn::Data::Struct(syn::DataStruct {
        fields: syn::Fields::Named(ref fields),
        ..
    }) = input.data else {
        abort_call_site!("`#[derive(PlanTraceObject)]` only supports structs with named fields.");
    };

    let spaces = util::get_fields_with_attribute(fields, "space");
    let post_scan_spaces = util::get_fields_with_attribute(fields, "post_scan");
    let parent = util::get_unique_field_with_attribute(fields, "parent");

    let trace_object_function =
        plan_trace_object_impl::generate_trace_object(&spaces, &parent, &ty_generics);
    let post_scan_object_function =
        plan_trace_object_impl::generate_post_scan_object(&post_scan_spaces, &parent, &ty_generics);
    let may_move_objects_function =
        plan_trace_object_impl::generate_may_move_objects(&spaces, &parent, &ty_generics);

    let output = quote! {
        impl #impl_generics crate::plan::PlanTraceObject #ty_generics for #ident #ty_generics #where_clause {
            #trace_object_function

            #post_scan_object_function

            #may_move_objects_function
        }
    };

    // Debug the output - use the following code to debug the generated code (when cargo exapand is not working)
    if DEBUG_MACRO_OUTPUT {
        use quote::ToTokens;
        println!("{}", output.to_token_stream());
    }

    output.into()
}
