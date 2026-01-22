use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{parse::Parse, Data, DeriveInput, Fields, FieldsNamed, Visibility};

#[derive(Debug)]
pub enum PortType {
    Input,
    Output,
}

#[derive(Debug)]
pub struct PortField {
    vis: Visibility,
    ident: syn::Ident,
    ty: syn::Type,
    port_type: PortType,
    len: Option<syn::Expr>,
}

pub struct Model {
    vis: Visibility,
    name: syn::Ident,
    generics: syn::Generics,
    fields: Vec<PortField>,
}

impl PortField {
    fn from_field(field: &syn::Field) -> syn::Result<Self> {
        let mut port_type = None;
        let mut len = None;

        for attr in &field.attrs {
            if attr.path().is_ident("input") {
                port_type = Some(PortType::Input);
                len = parse_len_attr(attr)?;
            } else if attr.path().is_ident("output") {
                port_type = Some(PortType::Output);
                len = parse_len_attr(attr)?;
            }
        }

        let port_type = port_type.ok_or_else(|| {
            syn::Error::new_spanned(field, "Field must be annotated with #[input] or #[output]")
        })?;

        if len.is_some() && matches!(field.ty, syn::Type::Array(_)) {
            return Err(syn::Error::new_spanned(
                field,
                "banked ports cannot be declared as arrays",
            ));
        }

        Ok(PortField {
            vis: field.vis.clone(),
            ident: field.ident.clone().unwrap(),
            ty: field.ty.clone(),
            port_type,
            len,
        })
    }
}

fn parse_len_attr(attr: &syn::Attribute) -> syn::Result<Option<syn::Expr>> {
    let meta = match &attr.meta {
        syn::Meta::Path(_) => return Ok(None),
        syn::Meta::List(list) => list,
        _ => {
            return Err(syn::Error::new_spanned(
                attr,
                "expected #[input] or #[input(len = ...)]",
            ))
        }
    };

    if meta.tokens.is_empty() {
        return Ok(None);
    }

    let nested_meta: syn::Meta = syn::parse2(meta.tokens.clone())?;
    match nested_meta {
        syn::Meta::NameValue(nv) if nv.path.is_ident("len") => Ok(Some(nv.value)),
        _ => Err(syn::Error::new_spanned(
            attr,
            "expected #[input(len = ...)] or #[output(len = ...)]",
        )),
    }
}

impl Parse for Model {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let input: DeriveInput = input.parse()?;

        let fields = match &input.data {
            Data::Struct(data) => match &data.fields {
                Fields::Named(FieldsNamed { named, .. }) => named,
                _ => {
                    return Err(syn::Error::new_spanned(
                        input,
                        "ReactorParts can only be derived for structs with named fields",
                    ))
                }
            },
            _ => {
                return Err(syn::Error::new_spanned(
                    input,
                    "ReactorParts can only be derived for structs",
                ))
            }
        };

        let fields = fields
            .into_iter()
            .map(PortField::from_field)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Model {
            vis: input.vis.clone(),
            name: input.ident,
            generics: input.generics,
            fields,
        })
    }
}

impl ToTokens for Model {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let vis = &self.vis;
        let struct_name = &self.name;
        let (impl_generics, ty_generics, where_clause) = self.generics.split_for_impl();

        // Generate the modified struct fields
        let struct_fields = self
            .fields
            .iter()
            .map(|PortField { vis, ident, ty, port_type, len }| {
            let dir = match port_type {
                PortType::Input => quote!(::boomerang::builder::Input),
                PortType::Output => quote!(::boomerang::builder::Output),
            };
            
            // Determine if this is an array type and handle appropriately
            match ty {
                syn::Type::Array(array) => {
                    let element_type = &array.elem;
                    let len_expr = &array.len;
                    quote!(#vis #ident: [::boomerang::builder::TypedPortKey<#element_type, #dir, ::boomerang::builder::Contained>; #len_expr])
                },
                _ if len.is_some() => {
                    quote!(#vis #ident: ::boomerang::builder::PortBank<#ty, #dir, ::boomerang::builder::Contained>)
                }
                _ => {
                    quote!(#vis #ident: ::boomerang::builder::TypedPortKey<#ty, #dir, ::boomerang::builder::Contained>)
                }
            }
        });

        // Generate implementation details as before
        let field_types = self.fields.iter().map(|f| {
            let ty = &f.ty;
            let dir = match f.port_type {
                PortType::Input => quote!(::boomerang::builder::Input),
                PortType::Output => quote!(::boomerang::builder::Output),
            };
            
            // Determine if this is an array type and handle appropriately
            match ty {
                syn::Type::Array(array) => {
                    let element_type = &array.elem;
                    let len_expr = &array.len;
                    quote!([::boomerang::builder::TypedPortKey<#element_type, #dir, ::boomerang::builder::Local>; #len_expr])
                },
                _ if f.len.is_some() => {
                    quote!(::boomerang::builder::PortBank<#ty, #dir, ::boomerang::builder::Local>)
                }
                _ => {
                    quote!(::boomerang::builder::TypedPortKey<#ty, #dir, ::boomerang::builder::Local>)
                }
            }
        });

        let local_names = self.fields.iter().map(|f| {
            if f.len.is_some() {
                format_ident!("{}_for_fn", f.ident)
            } else {
                f.ident.clone()
            }
        });

        let create_ports = self.fields.iter().map(
            |PortField {
                 vis: _,
                 ident,
                 ty,
                 port_type,
                 len,
             }| {
                let name_str = ident.to_string();
                let for_fn_ident = format_ident!("{}_for_fn", ident);
                let dir = match port_type {
                    PortType::Input => quote!(::boomerang::builder::Input),
                    PortType::Output => quote!(::boomerang::builder::Output),
                };
                
                // Determine if this is an array type and handle appropriately
                match ty {
                    syn::Type::Array(array) => {
                        let element_type = &array.elem;
                        let len_expr = &array.len;
                    
                    match port_type {
                            PortType::Input => quote! {
                                let #ident = builder.add_input_ports::<#element_type, #len_expr>(#name_str)?;
                            },
                            PortType::Output => quote! {
                                let #ident = builder.add_output_ports::<#element_type, #len_expr>(#name_str)?;
                            },
                        }
                    },
                    _ if len.is_some() => {
                        let len_expr = len.as_ref().expect("len expr");
                        match port_type {
                            PortType::Input => quote! {
                                let #ident = builder.add_input_bank::<#ty>(#name_str, #len_expr)?;
                                let #for_fn_ident = #ident.clone();
                            },
                            PortType::Output => quote! {
                                let #ident = builder.add_output_bank::<#ty>(#name_str, #len_expr)?;
                                let #for_fn_ident = #ident.clone();
                            },
                        }
                    }
                    _ => {
                        quote!(let #ident = builder.add_port::<#ty, #dir>(#name_str, None)?;)
                    }
                }
            },
        );

        let field_inits = self.fields.iter().map(|PortField {  ident, ty, len, ..  }| {
            match ty {
                syn::Type::Array(_) => {
                    quote!(#ident: std::array::from_fn(|i| #ident[i].contained()))
                }
                _ if len.is_some() => {
                    quote!(#ident: #ident.contained())
                }
                _ => {
                    quote!(#ident: #ident.contained())
                }
            }
        });

        let expanded = quote! {
            #vis struct #struct_name #impl_generics {
                #(#struct_fields,)*
            }

            impl #impl_generics ::boomerang::builder::ReactorPorts for #struct_name #ty_generics #where_clause {
                type Fields = (#(#field_types,)*);

                fn build_with<F, S>(f: F) -> impl ::boomerang::builder::Reactor<S, Ports = #struct_name #ty_generics>
                where
                    F: Fn(
                            &mut ::boomerang::builder::ReactorBuilderState<'_, S>,
                            Self::Fields,
                        ) -> Result<(), ::boomerang::builder::BuilderError>
                        + 'static,
                    S: ::boomerang::runtime::ReactorData,
                {
                    move |name: &str,
                     state: S,
                     parent: Option<::boomerang::builder::BuilderReactorKey>,
                     bank_info: Option<::boomerang::runtime::BankInfo>,
                     is_enclave: bool,
                     env: &mut ::boomerang::builder::EnvBuilder| {
                        let mut builder = env.add_reactor(name, parent, bank_info, state, is_enclave);
                        #(#create_ports)*
                        f(&mut builder, (#(#local_names,)*))?;
                        builder.finish()?;
                        Ok(#struct_name {
                            #(#field_inits,)*
                        })
                    }
                }
            }
        };

        tokens.extend(expanded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test1() {
        let input = r#"
#[derive(ReactorParts)]
struct Count {
    #[input]
    x: u32,
    #[output]
    y: bool,
}"#;

        let model = syn::parse_str::<Model>(input).unwrap();

        assert_eq!(model.fields.len(), 2);
        assert!(matches!(model.fields[0].port_type, PortType::Input));
        assert!(matches!(model.fields[1].port_type, PortType::Output));
    }

    #[test]
    fn test_generics() {
        let input = r#"
#[derive(ReactorParts)]
struct Generic<T: MyTrait, U> {
    #[input]
    x: T,
    #[output]
    y: U,
}"#;

        let model = syn::parse_str::<Model>(input).unwrap();
        assert!(!model.generics.params.is_empty());
    }

    #[test]
    fn test_arrays() {
        let input = r#"
#[derive(ReactorParts)]
struct Array {
    #[input] x: [u32; 10],
    #[output] y: [bool; 10],
}"#;

        let model = syn::parse_str::<Model>(input).unwrap();
        assert_eq!(model.fields.len(), 2);
        assert!(matches!(model.fields[0].port_type, PortType::Input));
        assert!(matches!(model.fields[1].port_type, PortType::Output));
    }

    #[test]
    fn test_banked_ports() {
        let input = r#"
#[derive(ReactorParts)]
struct Banked {
    #[input(len = 3)]
    x: u32,
    #[output(len = 4)]
    y: bool,
}"#;

        let model = syn::parse_str::<Model>(input).unwrap();
        let tokens = quote::quote!(#model).to_string();

        assert!(tokens.contains("add_input_bank"));
        assert!(tokens.contains("add_output_bank"));
    }
}
