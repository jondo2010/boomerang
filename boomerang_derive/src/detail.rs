//! Details of implementation for derive macros and attributes

use darling::{ast, util, FromDeriveInput, FromField, FromMeta};
use quote::{quote, ToTokens};

#[derive(Debug, Default, FromMeta)]
pub struct TimerField {
    pub offset: String,
    pub period: String,
}

#[derive(Debug, Default, Clone)]
pub struct StringList(Vec<String>);
impl std::ops::Deref for StringList {
    type Target = Vec<String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromMeta for StringList {
    fn from_list(v: &[syn::NestedMeta]) -> darling::Result<Self> {
        let mut strings = Vec::with_capacity(v.len());
        for nmi in v {
            if let syn::NestedMeta::Lit(syn::Lit::Str(ref string)) = *nmi {
                strings.push(string.value().clone());
            } else {
                return Err(darling::Error::unexpected_type("non-word").with_span(nmi));
            }
        }
        Ok(StringList(strings))
    }
}

#[derive(Debug, FromMeta)]
pub struct ReactionField {
    #[darling(default)]
    pub triggers: StringList,
    #[darling(default)]
    pub uses: StringList,
    #[darling(default)]
    pub effects: StringList,
}

#[derive(Debug, FromField)]
#[darling(attributes(reactor), forward_attrs(doc, cfg, allow))]
pub struct ReactorField {
    pub ident: Option<syn::Ident>,
    vis: syn::Visibility,
    ty: syn::Type,
    attrs: Vec<syn::Attribute>,

    #[darling(default)]
    pub input: bool,
    #[darling(default)]
    pub output: bool,
    #[darling(default)]
    pub timer: Option<TimerField>,
    #[darling(default)]
    pub reaction: Option<ReactionField>,
}

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(reactor), supports(struct_named))]
pub struct ReactorReceiver {
    pub ident: syn::Ident,
    pub generics: syn::Generics,
    // pub attrs: Vec<syn::Attribute>,
    pub data: ast::Data<util::Ignored, ReactorField>,
}

impl ToTokens for ReactorReceiver {
    /// # Panics
    /// This method panics if the field attributes input, output, timer are not mutually-exclusive.
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let type_ident = &self.ident;
        let (imp, ty, wher) = self.generics.split_for_impl();
        let fields = self
            .data
            .as_ref()
            .take_struct()
            .expect("Should never be enum")
            .fields;

        let mut impl_details = proc_macro2::TokenStream::new();

        for f in fields {
            let field_ident = &f.ident;
            let field_type = &f.ty;
            match (&f.input, &f.output, &f.timer, &f.reaction) {
                (false, false, None, None) => impl_details.extend(quote!(
                    #field_ident : Default::default(),
                )),
                (true, false, None, None) => todo!(),
                (false, true, None, None) => impl_details.extend(quote!(
                    #field_ident : Rc::new(RefCell::new(Port::default())),
                )),
                (false, false, Some(timer), None) => {
                    let offset =
                        syn::parse_str::<syn::Expr>(&timer.offset).expect("Invalid expression");
                    let period =
                        syn::parse_str::<syn::Expr>(&timer.period).expect("Invalid expression");
                    impl_details.extend(quote!(
                        #field_ident : Rc::new(RefCell::new(Trigger:: #ty ::new(
                            /* reactions:*/ vec![],
                            /* offset:*/ #offset,
                            /* period:*/ Some(#period),
                            /* is_physical:*/ false,
                            /* policy:*/ QueuingPolicy::NONE,
                        ))),
                        //let t = S::Timer::new();
                    ));
                }
                (false, false, None, Some(reaction)) => {
                    impl_details.extend(quote!(
                        #field_ident : {
                            let this_clone = this.clone();
                            let closure = Box::new(RefCell::new(move |sched: &mut S| {
                                HelloWorld::hello(&mut (*this_clone).borrow_mut(), sched);
                            }));
                            ::std::rc::Rc::new(Reaction::new(
                                "hello_reaction",
                                /* reactor */ closure,
                                /* index */ 0,
                                /* chain_id */ 0,
                                /* triggers */ vec![(this.borrow().output.clone(), vec![reply_in_trigger])],
                            ))
                        };
                    ));
                }
                _ => panic!(format!(
                    "Reactor attributes input/output/timer must be mutually exclusive: {:?}",
                    (&f.input, &f.output, &f.timer)
                )),
            }
        }

        tokens.extend(quote! {
            impl #imp #type_ident #ty #wher {
                pub fn schedule(this: &Rc<RefCell<Self>>, scheduler: &mut S) {
                }
                pub fn new() -> Self {
                    use crate::*;
                    println!("Poo!");
                    Self {
                        #impl_details
                        //..Default::default()
                    }
                }
            }
        })
    }
}

#[test]
fn test_reaction_field() {
    let input = syn::parse_str(
        r#"
    #[derive(Reactor)]
    pub struct Foo {
        #[reactor(reaction(
            triggers("tim1", "hello1.x"),
            uses(),
        ))]
        hello2: u32,
    }
    "#,
    )
    .unwrap();
    let receiver: ReactorReceiver = ReactorReceiver::from_derive_input(&input).unwrap();
    let fields = &receiver.data.take_struct().unwrap().fields;
    dbg!(&fields[0]);
    assert_eq!(fields.len(), 1);
    assert!(fields[0].reaction.is_some());
}

#[test]
fn test_parse() {
    let good_input = r#"
    #[derive(Reactor)]
    pub struct Foo {
        //#[reactor(input)]
        //bar: bool,

        //#[reactor(output)]
        //baz: i64,

        //#[reactor(timer(offset="Duration::from_millis(100)", period="Duration::from_millis(1000)"))]
        //foo: u32,

    }"#;
    let parsed = syn::parse_str(good_input).unwrap();
    let receiver = ReactorReceiver::from_derive_input(&parsed).unwrap();
    // assert_eq!(receiver.timers[0].offset, "Duration::from_millis(100)");
    dbg!(receiver);
}
