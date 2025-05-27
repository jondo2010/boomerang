use std::time::Duration;

use quote::ToTokens;
use syn::{parse::Parse, Ident};

/// Parse a reaction definition like:
///
/// ```no_run
/// timer! { <name>(<offset>, <period>) };
/// ```
///
/// The <period>, which is optional, specifies the time interval between timer events. The <offset>, which is also
/// optional, specifies the (logical) time interval between when the program starts executing and the first timer event.
/// If no period is given, then the timer event occurs only once. If neither an offset nor a period is specified, then
/// one timer event occurs at program start, simultaneous with the startup event.
///
/// ## Example
/// ```no_run
/// // A timer that triggers after 10 seconds and then every 50 milliseconds.
/// timer! { t1(10 sec, 50 msec) };
/// ```
#[derive(Debug)]
pub struct Model {
    name: Ident,
    offset: Option<Duration>,
    period: Duration,
}

impl Parse for Model {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // Parst den Timer-Namen (z.B. "t1")
        let name: Ident = input.parse()?;

        // Es sollte eine öffnende Klammer folgen
        let content;
        syn::parenthesized!(content in input);

        // Parse the optional offset and period (comma delimited)
        let mut offset = None;
        let mut period = None;

        // Parse the first duration
        if !content.is_empty() {
            let duration = content.parse::<crate::time::Dur>()?.0;

            // If there's a comma, this is the offset and we expect a period next
            if content.peek(syn::Token![,]) {
                content.parse::<syn::Token![,]>()?; // Consume the comma
                offset = Some(duration);

                // Parse the period (required if we had an offset)
                if content.is_empty() {
                    return Err(content.error("Expected a period after the offset"));
                }
                period = Some(content.parse::<crate::time::Dur>()?.0);
            } else {
                // If no comma, this is the period
                period = Some(duration);
            }
        }

        // Check for trailing content that shouldn't be there
        if !content.is_empty() {
            return Err(content.error("Unexpected trailing content in timer definition"));
        }

        // Ensure we have a period
        let period = period.ok_or_else(|| input.error("A timer must have at least a period"))?;

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

        // Convert periods to milliseconds for the runtime Duration type
        let period_ms = self.period.as_millis() as i64;

        let mut timer_spec = quote::quote! {
            ::boomerang::prelude::TimerSpec::default()
                .with_period(::boomerang::runtime::Duration::milliseconds(#period_ms))
        };

        // Add offset if present
        if let Some(offset) = self.offset {
            let offset_ms = offset.as_millis() as i64;
            timer_spec = quote::quote! {
                ::boomerang::prelude::TimerSpec::default()
                    .with_offset(::boomerang::runtime::Duration::milliseconds(#offset_ms))
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
    fn parse_timer() {
        let model: Model = parse_quote! { t1(10 sec, 50 msec) };
        assert_eq!(model.name, "t1");
        assert_eq!(model.offset.unwrap().as_millis(), 10000);
        assert_eq!(model.period.as_millis(), 50);
    }

    #[test]
    fn parse_timer_without_offset() {
        let model: Model = parse_quote! { t2(100 msec) };
        assert_eq!(model.name, "t2");
        assert!(model.offset.is_none());
        assert_eq!(model.period.as_millis(), 100);
    }
}
