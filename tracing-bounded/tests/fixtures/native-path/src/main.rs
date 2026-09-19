//! Fresh-process upstream audit, NOT the production bounded subscriber.

mod allocation;
mod output;
mod sites;

use output::{AuditSubscriber, Snapshot};
use std::sync::Barrier;

struct MustNotFormat;

impl std::fmt::Debug for MustNotFormat {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        panic!("formatter was invoked")
    }
}

fn concurrent(first: fn(), second: fn()) {
    let barrier = Barrier::new(2);
    let barrier = &barrier;
    std::thread::scope(|scope| {
        let workers = [first, second].map(|emit| {
            scope.spawn(move || {
                allocation::prepare();
                barrier.wait();
                allocation::measure(emit)
            })
        });
        for worker in workers {
            assert_eq!(
                worker.join().expect("audit worker panicked"),
                allocation::Counts::default()
            );
        }
    });
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [case] = args.as_slice() else {
        eprintln!("expected exactly one audit scenario");
        std::process::exit(2);
    };
    if case == "allocator-control" {
        allocation::prepare();
        assert_eq!(allocation::measure(|| {}), allocation::Counts::default());
        let counts = allocation::measure(|| drop(std::hint::black_box(Box::new(42_u64))));
        assert!(
            counts.allocations > 0 && counts.deallocations > 0,
            "observer missed allocation/deallocation: {counts:?}"
        );
        println!("allocator-control: {counts:?}");
        return;
    }
    let expected = match case.as_str() {
        "first-use" => Snapshot {
            accepted: 1,
            rejected: 0,
            sequences: 1,
        },
        "repeated" => Snapshot {
            accepted: 1024,
            rejected: 0,
            sequences: 1,
        },
        "different-sites" => Snapshot {
            accepted: 2,
            rejected: 0,
            sequences: 3,
        },
        "same-site" => Snapshot {
            accepted: 2,
            rejected: 0,
            sequences: 1,
        },
        "filtered" => Snapshot {
            accepted: 0,
            rejected: 0,
            sequences: 0,
        },
        "unsupported" => Snapshot {
            accepted: 0,
            rejected: 1,
            sequences: 0,
        },
        _ => {
            eprintln!("unknown audit scenario: {case}");
            std::process::exit(2);
        }
    };
    tracing::subscriber::set_global_default(AuditSubscriber).expect("one global subscriber");
    allocation::prepare();
    let counts = match case.as_str() {
        "different-sites" => {
            concurrent(sites::first, sites::second);
            allocation::Counts::default()
        }
        "same-site" => {
            concurrent(sites::first, sites::first);
            allocation::Counts::default()
        }
        _ => allocation::measure(|| emit(case)),
    };
    assert_eq!(
        counts,
        allocation::Counts::default(),
        "emission allocated/deallocated for {case}"
    );
    let actual = output::snapshot();
    assert_eq!(actual, expected, "native subscriber output for {case}");
    println!("{case}: {actual:?}; emission allocations/deallocations=0");
}

fn emit(case: &str) {
    match case {
        "first-use" => sites::first(),
        "repeated" => {
            for _ in 0..1024 {
                sites::first();
            }
        }
        "filtered" => sites::filtered(),
        "unsupported" => sites::rejected(&MustNotFormat),
        _ => unreachable!(),
    }
}
