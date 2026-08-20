#![cfg(feature = "rerun")]

use boomerang::rerun::{RerunSessionBuilder, SinkConfig};

#[test]
fn memory_session_starts_enabled_with_empty_failure_counts() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .source_id("test-process")
        .sink(SinkConfig::Memory)
        .build()
        .unwrap();

    assert!(session.is_enabled());
    assert_eq!(session.error_count(), 0);
    assert_eq!(session.skipped_count(), 0);
    assert_eq!(session.source_id(), "test-process");
}

#[test]
fn default_source_ids_are_generated_per_session() {
    let first = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();
    let second = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();

    assert_ne!(first.source_id(), second.source_id());
}

#[test]
fn memory_session_flush_is_idempotent_smoke_test() {
    let session = RerunSessionBuilder::new("boomerang-rerun-test")
        .build()
        .unwrap();

    session.flush();
    session.flush();

    assert!(session.is_enabled());
    assert_eq!(session.error_count(), 0);
    assert_eq!(session.skipped_count(), 0);
}
