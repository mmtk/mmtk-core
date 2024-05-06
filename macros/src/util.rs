use proc_macro_error::abort;
use syn::{spanned::Spanned, Attribute, Field, FieldsNamed};

pub fn get_field_attribute<'f>(field: &'f Field, attr_name: &str) -> Option<&'f Attribute> {
    let attrs = field
        .attrs
        .iter()
        .filter(|a| a.path().is_ident(attr_name))
        .collect::<Vec<_>>();
    if attrs.len() > 1 {
        let second_attr = attrs.get(1).unwrap();
        abort! { second_attr.path().span(), "Duplicated attribute: #[{}]", attr_name }
    };

    attrs.first().cloned()
}

pub fn get_fields_with_attribute<'f>(fields: &'f FieldsNamed, attr_name: &str) -> Vec<&'f Field> {
    fields
        .named
        .iter()
        .filter(|f| get_field_attribute(f, attr_name).is_some())
        .collect::<Vec<_>>()
}

pub fn get_unique_field_with_attribute<'f>(
    fields: &'f FieldsNamed,
    attr_name: &str,
) -> Option<&'f Field> {
    let mut result = None;

    for field in fields.named.iter() {
        if let Some(attr) = get_field_attribute(field, attr_name) {
            if result.is_none() {
                result = Some(field);
                continue;
            } else {
                let span = attr.path().span();
                abort! { span, "At most one field in a struct can have the #[{}] attribute.", attr_name };
            }
        }
    }

    result
}
