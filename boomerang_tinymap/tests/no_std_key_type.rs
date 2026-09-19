#![no_std]

extern crate alloc;
extern crate std;

boomerang_tinymap::key_type!(DownstreamKey);

#[test]
fn key_type_expands_for_a_no_std_alloc_consumer_without_format_import() {
    use core::str::FromStr;

    assert_eq!(
        DownstreamKey::from_str("DownstreamKey(7)").unwrap(),
        DownstreamKey::new(7)
    );
    assert_eq!(
        DownstreamKey::from_str("invalid").unwrap_err(),
        "Invalid format for DownstreamKey: invalid"
    );
}
