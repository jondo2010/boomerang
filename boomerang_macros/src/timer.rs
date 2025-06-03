use std::time::Duration;

use quote::ToTokens;
use syn::{parse::Parse, Ident};

/// Parse a reaction definition like:
///
/// ```rust,ignore
/// timer! { <name>(<offset>, <period>) };
/// ```
///
/// The <offset>, which is optional, specifies the (logical) time interval between when the program starts executing and the first timer event.
/// The <period>, which is also optional, specifies the time interval between timer events.
/// If no period is given, then the timer event occurs only once. If neither an offset nor a period is specified, then
/// one timer event occurs at program start, simultaneous with the startup event.
///
/// ## Example
/// ```rust,ignore
/// // A timer that triggers after 10 seconds and then every 50 milliseconds.
/// timer! { t1(10 sec, 50 msec) };
///
/// // A one-shot timer that triggers after 100 milliseconds.
/// timer! { t2(100 msec) };
///
/// // A one-second periodic timer with no offset.
/// timer! { t3(0, 1 s) };
///
/// // A one-shot timer that triggers at program start.
/// timer! { t3() };
/// ```
#[derive(Debug)]
pub struct Model {
    name: Ident,
    offset: Option<Duration>,
    period: Option<Duration>,
}

impl Parse for Model {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // Parse the timer name, which should be an identifier
        let name: Ident = input.parse()?;

        let content;
        syn::parenthesized!(content in input);

        // Parse the optional offset and period (comma delimited)
        let mut offset = None;
        let mut period = None;

        // Parse the first duration (offset)
        if !content.is_empty() {
            let duration = content.parse::<crate::time::Dur>()?.0;
            offset = Some(duration);

            // If there's a comma, parse the period next
            if content.peek(syn::Token![,]) {
                content.parse::<syn::Token![,]>()?; // Consume the comma

                // Parse the period (required if we had a comma)
                if content.is_empty() {
                    return Err(content.error("Expected a period after the comma"));
                }
                period = Some(content.parse::<crate::time::Dur>()?.0);
            }
        }

        // Check for trailing content that shouldn't be there
        if !content.is_empty() {
            return Err(content.error("Unexpected trailing content in timer definition"));
        }

        Ok(Self {
            name,
            offset,
            period,
        })
    }
}

impl ToTokens for Model {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let name = &self.name;
        let name_str = name.to_string();

        let mut timer_spec = quote::quote! {
            ::boomerang::prelude::TimerSpec::default()
        };

        // Add offset if present
        if let Some(offset) = self.offset {
            let offset_ms = offset.as_millis() as i64;
            timer_spec = quote::quote! {
                #timer_spec
                    .with_offset(::boomerang::runtime::Duration::milliseconds(#offset_ms))
            };
        }

        // Add period if present
        if let Some(period) = self.period {
            let period_ms = period.as_millis() as i64;
            timer_spec = quote::quote! {
                #timer_spec
                    .with_period(::boomerang::runtime::Duration::milliseconds(#period_ms))
            };
        }

        tokens.extend(quote::quote! {
            let #name = builder.add_timer(
                #name_str,
                #timer_spec,
            )?;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn parse_timer_with_offset_and_period() {
        let model: Model = parse_quote! { t1(10 sec, 50 msec) };
        assert_eq!(model.name, "t1");
        assert_eq!(model.offset.unwrap().as_millis(), 10000);
        assert_eq!(model.period.unwrap().as_millis(), 50);
    }

    #[test]
    fn parse_timer_with_offset_only() {
        let model: Model = parse_quote! { t2(100 msec) };
        assert_eq!(model.name, "t2");
        assert_eq!(model.offset.unwrap().as_millis(), 100);
        assert!(model.period.is_none());
    }

    #[test]
    fn parse_timer_empty() {
        let model: Model = parse_quote! { t3() };
        assert_eq!(model.name, "t3");
        assert!(model.offset.is_none());
        assert!(model.period.is_none());
    }
}
