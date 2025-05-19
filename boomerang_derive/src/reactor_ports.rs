use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
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

        for attr in &field.attrs {
            if attr.path().is_ident("input") {
                port_type = Some(PortType::Input);
            } else if attr.path().is_ident("output") {
                port_type = Some(PortType::Output);
            }
        }

        let port_type = port_type.ok_or_else(|| {
            syn::Error::new_spanned(field, "Field must be annotated with #[input] or #[output]")
        })?;

        Ok(PortField {
            vis: field.vis.clone(),
            ident: field.ident.clone().unwrap(),
            ty: field.ty.clone(),
            port_type,
        })
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
        let struct_fields = self.fields.iter().map(|PortField { vis, ident, ty, port_type }| {
            let dir = match *port_type {
                PortType::Input => quote!(::boomerang::builder::Input),
                PortType::Output => quote!(::boomerang::builder::Output),
            };
            quote!(#vis #ident: ::boomerang::builder::TypedPortKey<#ty, #dir, ::boomerang::builder::Contained>)
        });

        // Generate implementation details as before
        let field_types = self.fields.iter().map(|f| {
            let ty = &f.ty;
            let dir = match f.port_type {
                PortType::Input => quote!(::boomerang::builder::Input),
                PortType::Output => quote!(::boomerang::builder::Output),
            };
            quote!(::boomerang::builder::TypedPortKey<#ty, #dir, ::boomerang::builder::Local>)
        });

        let local_names = self.fields.iter().map(|f| &f.ident);

        let create_ports = self.fields.iter().map(
            |PortField {
                 vis: _,
                 ident,
                 ty,
                 port_type,
             }| {
                let name_str = ident.to_string();
                let dir = match *port_type {
                    PortType::Input => quote!(::boomerang::builder::Input),
                    PortType::Output => quote!(::boomerang::builder::Output),
                };
                quote!(let #ident = builder.add_port::<#ty, #dir>(#name_str, None)?;)
            },
        );

        let field_inits = self.fields.iter().map(|f| {
            let name = &f.ident;
            quote!(#name: #name.contained())
        });

        let expanded = quote! {
            #vis struct #struct_name #impl_generics {
                #(#struct_fields,)*
            }

            impl #impl_generics ::boomerang::builder::ReactorPorts for #struct_name #ty_generics #where_clause {
                type Fields = (#(#field_types,)*);

                fn build_with<F, S>(f: F) -> impl ::boomerang::builder::Reactor2<S, Ports = #struct_name #ty_generics>
                where
                    F: FnOnce(
                            &mut ::boomerang::builder::ReactorBuilderState<'_, S>,
                            Self::Fields,
                        ) -> Result<(), ::boomerang::builder::BuilderError>
                        + 'static,
                    S: ::boomerang::runtime::ReactorData,
                {
                    |name: &str,
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
    fn test_tokens() {
        let input = r#"
#[derive(ReactorPorts)]
struct ScalePorts {
    #[input]
    x: u32,
    #[output]
    y: u32,
}"#;

        let model = syn::parse_str::<Model>(input).unwrap();
        let tokens = quote!(#model);
        let output = tokens.to_string();

        assert!(output.contains("impl ReactorPorts for ScalePorts"));
        assert!(output.contains(
            "type Fields = (TypedPortKey<u32, Input, Local>, TypedPortKey<u32, Output, Local>)"
        ));
    }

    #[test]
    fn test_struct_modification() {
        let input = r#"
#[derive(ReactorPorts)]
struct ScalePorts {
    #[input]
    x: u32,
    #[output]
    y: u32,
}"#;

        let model = syn::parse_str::<Model>(input).unwrap();
        let tokens = quote!(#model);
        let output = tokens.to_string();

        // Check that the struct is properly modified
        assert!(output.contains("struct ScalePorts"));
        assert!(output.contains("x : TypedPortKey < u32 , Input , Contained >"));
        assert!(output.contains("y : TypedPortKey < u32 , Output , Contained >"));

        // Check that the implementation is still correct
        assert!(output.contains("impl ReactorPorts for ScalePorts"));
    }
}
