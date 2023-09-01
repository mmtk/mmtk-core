extern crate proc_macro;
extern crate proc_macro_error;
extern crate quote;
extern crate syn;

use proc_macro::TokenStream;
use proc_macro_error::proc_macro_error;
use syn::parse_macro_input;
use syn::DeriveInput;

mod has_spaces_impl;
mod plan_trace_object_impl;
mod util;

const DEBUG_MACRO_OUTPUT: bool = false;

/// This macro will generate an implementation of `HasSpaces` for a plan or any structs that
/// contain spaces, including `Gen`, `CommonPlan` and `BasePlan`.
///
/// The `HasSpaces` trait is responsible for enumerating spaces in a struct.  When using this
/// derive macro, the user should do the following.
///
/// * Make sure the struct has a generic type parameter named `VM` which requires `VMBinding`.
///   For example, `struct MyPlan<VM: VMBinding>` will work.
/// * Add `#[space]` for each space field in the struct.
/// * Add `#[parent]` to the field that contain more space fields.  This attribute is usually
///   added to `Gen`, `CommonPlan` or `BasePlan` fields.  There can be at most one parent in
///   a struct.
#[proc_macro_error]
#[proc_macro_derive(HasSpaces, attributes(space, parent))]
pub fn derive_has_spaces(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let output = has_spaces_impl::derive(input);

    output.into()
}

/// The macro will generate an implementation of `PlanTraceObject` for the plan. With
/// `PlanTraceObject`, the plan will be able to use `PlanProcessEdges` for GC tracing.
///
/// The user should add `#[space]` and `#[parent]` attributes to fields as specified by the
/// `HasSpaces` trait.  When using this derive macro, all spaces must implement the
/// `PolicyTraceObject` trait.  The generated `trace_object` method will check for spaces in the
/// current plan and, if the object is not in any of them, check for plans in the parent struct.
/// The parent struct must also implement the `PlanTraceObject` trait.
///
/// In addition, the user can add the following attributes to fields in order to control the
/// behavior of the generated `trace_object` method.
///
/// * Add `#[copy_semantics(CopySemantics::X)]` to a space field to specify that when tracing
///   objects in that space, `Some(CopySemantics::X)` will be passed to the `Space::trace_object`
///   method as the `copy` argument.
/// * Add `#[post_scan]` to any space field that has some policy-specific `post_scan_object()`. For
///   objects in those spaces, `post_scan_object()` in the policy will be called after
///   `VM::VMScanning::scan_object()`.
#[proc_macro_error]
#[proc_macro_derive(PlanTraceObject, attributes(space, parent, copy_semantics, post_scan))]
pub fn derive_plan_trace_object(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let output = plan_trace_object_impl::derive(input);

    // Debug the output - use the following code to debug the generated code (when cargo exapand is not working)
    if DEBUG_MACRO_OUTPUT {
        use quote::ToTokens;
        println!("{}", output.to_token_stream());
    }

    output.into()
}
