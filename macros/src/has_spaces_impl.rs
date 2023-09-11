use proc_macro2::TokenStream as TokenStream2;
use proc_macro_error::abort_call_site;
use quote::quote;
use syn::{DeriveInput, Field};

use crate::util;

pub(crate) fn derive(input: DeriveInput) -> TokenStream2 {
    let ident = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let syn::Data::Struct(syn::DataStruct {
        fields: syn::Fields::Named(ref fields),
        ..
    }) = input.data
    else {
        abort_call_site!("`#[derive(HasSpaces)]` only supports structs with named fields.");
    };

    let spaces = util::get_fields_with_attribute(fields, "space");
    let parent = util::get_unique_field_with_attribute(fields, "parent");

    let items = generate_impl_items(&spaces, &parent);

    quote! {
        impl #impl_generics crate::plan::HasSpaces for #ident #ty_generics #where_clause {
            type VM = VM;

            #items
        }
    }
}

pub(crate) fn generate_impl_items<'a>(
    space_fields: &[&'a Field],
    parent_field: &Option<&'a Field>,
) -> TokenStream2 {
    // Currently we implement callback-style visitor methods.
    // Iterators should be more powerful, but is more difficult to implement.

    let mut space_visitors = vec![];
    let mut space_visitors_mut = vec![];

    for f in space_fields {
        let f_ident = f.ident.as_ref().unwrap();

        let visitor = quote! {
            __func(&self.#f_ident);
        };

        let visitor_mut = quote! {
            __func(&mut self.#f_ident);
        };

        space_visitors.push(visitor);
        space_visitors_mut.push(visitor_mut);
    }

    let (parent_visitor, parent_visitor_mut) = if let Some(f) = parent_field {
        let f_ident = f.ident.as_ref().unwrap();
        let visitor = quote! {
            self.#f_ident.for_each_space(__func)
        };
        let visitor_mut = quote! {
            self.#f_ident.for_each_space_mut(__func)
        };
        (visitor, visitor_mut)
    } else {
        (quote! {}, quote! {})
    };

    quote! {
        fn for_each_space(&self, __func: &mut dyn FnMut(&dyn Space<VM>)) {
            #(#space_visitors)*
            #parent_visitor
        }

        fn for_each_space_mut(&mut self, __func: &mut dyn FnMut(&mut dyn Space<VM>)) {
            #(#space_visitors_mut)*
            #parent_visitor_mut
        }
    }
}
