//! Duration value parsing.
//!
//! A time value is given with units (unless the value is 0, in which case the units can be omitted). The allowable units are:
//! For nanoseconds: ns, nsec, or nsecs
//! For microseconds: us, usec, or usecs
//! For milliseconds: ms, msec, or msecs
//! For seconds: s, sec, secs, second, or seconds
//! For minutes: min, minute, mins, or minutes
//! For hours: h, hour, or hours
//! For days: d, day, or days
//! For weeks: week or weeks
//!
//! Examples: `10 sec`, `50 msec`

use syn::parse::Parse;

// Konstanten für Zeitumrechnungen
const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 60 * 60;
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
const SECONDS_PER_WEEK: u64 = 7 * 24 * 60 * 60;

mod kw {
    syn::custom_keyword!(ns);
    syn::custom_keyword!(nsec);
    syn::custom_keyword!(nsecs);

    syn::custom_keyword!(us);
    syn::custom_keyword!(usec);
    syn::custom_keyword!(usecs);

    syn::custom_keyword!(ms);
    syn::custom_keyword!(msec);
    syn::custom_keyword!(msecs);

    syn::custom_keyword!(s);
    syn::custom_keyword!(sec);
    syn::custom_keyword!(secs);
    syn::custom_keyword!(second);
    syn::custom_keyword!(seconds);

    syn::custom_keyword!(min);
    syn::custom_keyword!(minute);
    syn::custom_keyword!(mins);
    syn::custom_keyword!(minutes);

    syn::custom_keyword!(h);
    syn::custom_keyword!(hour);
    syn::custom_keyword!(hours);

    syn::custom_keyword!(d);
    syn::custom_keyword!(day);
    syn::custom_keyword!(days);

    syn::custom_keyword!(week);
    syn::custom_keyword!(weeks);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dur(pub std::time::Duration);

impl From<std::time::Duration> for Dur {
    fn from(duration: std::time::Duration) -> Self {
        Self(duration)
    }
}

// Parse a duration string like "10 sec" or "50 msec"
impl Parse for Dur {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let value: syn::LitInt = input.parse()?;
        let value_u64 = value.base10_parse::<u64>()?;

        let lookahead = input.lookahead1();
        if lookahead.peek(kw::ns) {
            input.parse::<kw::ns>()?;
            Ok(Dur(std::time::Duration::from_nanos(value_u64)))
        } else if lookahead.peek(kw::nsec) {
            input.parse::<kw::nsec>()?;
            Ok(Dur(std::time::Duration::from_nanos(value_u64)))
        } else if lookahead.peek(kw::nsecs) {
            input.parse::<kw::nsecs>()?;
            Ok(Dur(std::time::Duration::from_nanos(value_u64)))
        } else if lookahead.peek(kw::us) {
            input.parse::<kw::us>()?;
            Ok(Dur(std::time::Duration::from_micros(value_u64)))
        } else if lookahead.peek(kw::usec) {
            input.parse::<kw::usec>()?;
            Ok(Dur(std::time::Duration::from_micros(value_u64)))
        } else if lookahead.peek(kw::usecs) {
            input.parse::<kw::usecs>()?;
            Ok(Dur(std::time::Duration::from_micros(value_u64)))
        } else if lookahead.peek(kw::ms) {
            input.parse::<kw::ms>()?;
            Ok(Dur(std::time::Duration::from_millis(value_u64)))
        } else if lookahead.peek(kw::msec) {
            input.parse::<kw::msec>()?;
            Ok(Dur(std::time::Duration::from_millis(value_u64)))
        } else if lookahead.peek(kw::msecs) {
            input.parse::<kw::msecs>()?;
            Ok(Dur(std::time::Duration::from_millis(value_u64)))
        } else if lookahead.peek(kw::s) {
            input.parse::<kw::s>()?;
            Ok(Dur(std::time::Duration::from_secs(value_u64)))
        } else if lookahead.peek(kw::sec) {
            input.parse::<kw::sec>()?;
            Ok(Dur(std::time::Duration::from_secs(value_u64)))
        } else if lookahead.peek(kw::secs) {
            input.parse::<kw::secs>()?;
            Ok(Dur(std::time::Duration::from_secs(value_u64)))
        } else if lookahead.peek(kw::second) {
            input.parse::<kw::second>()?;
            Ok(Dur(std::time::Duration::from_secs(value_u64)))
        } else if lookahead.peek(kw::seconds) {
            input.parse::<kw::seconds>()?;
            Ok(Dur(std::time::Duration::from_secs(value_u64)))
        } else if lookahead.peek(kw::min) {
            input.parse::<kw::min>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_MINUTE,
            )))
        } else if lookahead.peek(kw::minute) {
            input.parse::<kw::minute>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_MINUTE,
            )))
        } else if lookahead.peek(kw::mins) {
            input.parse::<kw::mins>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_MINUTE,
            )))
        } else if lookahead.peek(kw::minutes) {
            input.parse::<kw::minutes>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_MINUTE,
            )))
        } else if lookahead.peek(kw::h) {
            input.parse::<kw::h>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_HOUR,
            )))
        } else if lookahead.peek(kw::hour) {
            input.parse::<kw::hour>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_HOUR,
            )))
        } else if lookahead.peek(kw::hours) {
            input.parse::<kw::hours>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_HOUR,
            )))
        } else if lookahead.peek(kw::d) {
            input.parse::<kw::d>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_DAY,
            )))
        } else if lookahead.peek(kw::day) {
            input.parse::<kw::day>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_DAY,
            )))
        } else if lookahead.peek(kw::days) {
            input.parse::<kw::days>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_DAY,
            )))
        } else if lookahead.peek(kw::week) {
            input.parse::<kw::week>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_WEEK,
            )))
        } else if lookahead.peek(kw::weeks) {
            input.parse::<kw::weeks>()?;
            Ok(Dur(std::time::Duration::from_secs(
                value_u64 * SECONDS_PER_WEEK,
            )))
        } else {
            Err(lookahead.error())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;
    use syn::parse_quote;

    #[test]
    fn test_nanoseconds() {
        let parse: Dur = parse_quote!(10 ns);
        assert_eq!(parse.0, StdDuration::from_nanos(10));
        let parse: Dur = parse_quote!(20 nsec);
        assert_eq!(parse.0, StdDuration::from_nanos(20));
        let parse: Dur = parse_quote!(30 nsecs);
        assert_eq!(parse.0, StdDuration::from_nanos(30));
    }

    #[test]
    fn test_microseconds() {
        let parse: Dur = parse_quote!(10 us);
        assert_eq!(parse.0, StdDuration::from_micros(10));
        let parse: Dur = parse_quote!(20 usec);
        assert_eq!(parse.0, StdDuration::from_micros(20));
        let parse: Dur = parse_quote!(30 usecs);
        assert_eq!(parse.0, StdDuration::from_micros(30));
    }

    #[test]
    fn test_milliseconds() {
        let parse: Dur = parse_quote!(10 ms);
        assert_eq!(parse.0, StdDuration::from_millis(10));
        let parse: Dur = parse_quote!(20 msec);
        assert_eq!(parse.0, StdDuration::from_millis(20));
        let parse: Dur = parse_quote!(30 msecs);
        assert_eq!(parse.0, StdDuration::from_millis(30));
    }

    #[test]
    fn test_seconds() {
        let parse: Dur = parse_quote!(10 s);
        assert_eq!(parse.0, StdDuration::from_secs(10));
        let parse: Dur = parse_quote!(20 sec);
        assert_eq!(parse.0, StdDuration::from_secs(20));
        let parse: Dur = parse_quote!(30 secs);
        assert_eq!(parse.0, StdDuration::from_secs(30));
        let parse: Dur = parse_quote!(40 second);
        assert_eq!(parse.0, StdDuration::from_secs(40));
        let parse: Dur = parse_quote!(50 seconds);
        assert_eq!(parse.0, StdDuration::from_secs(50));
    }

    #[test]
    fn test_minutes() {
        let parse: Dur = parse_quote!(10 min);
        assert_eq!(parse.0, StdDuration::from_secs(10 * SECONDS_PER_MINUTE));
        let parse: Dur = parse_quote!(20 mins);
        assert_eq!(parse.0, StdDuration::from_secs(20 * SECONDS_PER_MINUTE));
        let parse: Dur = parse_quote!(30 minute);
        assert_eq!(parse.0, StdDuration::from_secs(30 * SECONDS_PER_MINUTE));
        let parse: Dur = parse_quote!(40 minutes);
        assert_eq!(parse.0, StdDuration::from_secs(40 * SECONDS_PER_MINUTE));
    }

    #[test]
    fn test_hours() {
        let parse: Dur = parse_quote!(1 h);
        assert_eq!(parse.0, StdDuration::from_secs(SECONDS_PER_HOUR));
        let parse: Dur = parse_quote!(2 hour);
        assert_eq!(parse.0, StdDuration::from_secs(2 * SECONDS_PER_HOUR));
        let parse: Dur = parse_quote!(3 hours);
        assert_eq!(parse.0, StdDuration::from_secs(3 * SECONDS_PER_HOUR));
    }

    #[test]
    fn test_days() {
        let parse: Dur = parse_quote!(1 d);
        assert_eq!(parse.0, StdDuration::from_secs(SECONDS_PER_DAY));
        let parse: Dur = parse_quote!(2 day);
        assert_eq!(parse.0, StdDuration::from_secs(2 * SECONDS_PER_DAY));
        let parse: Dur = parse_quote!(3 days);
        assert_eq!(parse.0, StdDuration::from_secs(3 * SECONDS_PER_DAY));
    }

    #[test]
    fn test_weeks() {
        let parse: Dur = parse_quote!(1 week);
        assert_eq!(parse.0, StdDuration::from_secs(SECONDS_PER_WEEK));
        let parse: Dur = parse_quote!(2 weeks);
        assert_eq!(parse.0, StdDuration::from_secs(2 * SECONDS_PER_WEEK));
    }

    #[test]
    #[should_panic]
    fn test_invalid_unit() {
        // Diese Einheit existiert nicht und sollte fehlschlagen
        let _parse: Dur = parse_quote!(10 invalid);
    }

    #[test]
    #[should_panic]
    fn test_missing_unit() {
        // Fehlende Einheit sollte fehlschlagen
        let _parse: Dur = parse_quote!(10);
    }

    #[test]
    fn test_from_std_duration() {
        let std_duration = StdDuration::from_secs(42);
        let duration = Dur::from(std_duration);
        assert_eq!(duration.0, std_duration);
    }
}
