#![no_std]

boomerang_tinymap::key_type!(pub ReactorIndex);

#[test]
fn key_comparisons_evaluate_in_const_context() {
    const LOW: ReactorIndex = ReactorIndex::new(0);
    const HIGH: ReactorIndex = ReactorIndex::new(u32::MAX);
    const {
        assert!(LOW.const_eq(LOW));
        assert!(!LOW.const_eq(HIGH));
        assert!(!HIGH.const_eq(LOW));
        assert!(matches!(LOW.const_cmp(HIGH), core::cmp::Ordering::Less));
        assert!(matches!(LOW.const_cmp(LOW), core::cmp::Ordering::Equal));
        assert!(matches!(HIGH.const_cmp(LOW), core::cmp::Ordering::Greater));
    }
}
