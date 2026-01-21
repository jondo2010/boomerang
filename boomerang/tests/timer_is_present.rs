//! Ported from <https://github.com/lf-lang/lingua-franca/blob/master/test/Cpp/src/TimerIsPresent.lf>

use boomerang::prelude::*;

#[reactor]
fn Main() -> impl Reactor {
    timer! { t1(0 s, 50 msec) };
    timer! { t2(33 msec, 33 msec) };

    reaction! {
        (startup) t1, t2 {
            println!("Startup reaction triggered.");
            assert!(startup.is_present(ctx), "Startup is not present.");
            assert!(t1.is_present(ctx), "t1 is not present at startup.");
            assert!(!t2.is_present(ctx), "t2 is present at startup.");
        }
    }

    reaction! {
        (t1, t2) {
            if t1.is_present(ctx) && t2.is_present(ctx) {
                panic!("t1 and t2 are both present.");
            }

            if !t1.is_present(ctx) && !t2.is_present(ctx){
                panic!("Either t1 or t2 should be present.");
            }
        }
    }

    reaction! {
        (shutdown) t1, t2 {
            assert!(shutdown.is_present(ctx), "Shutdown is not present.");
            assert!(t1.is_present(ctx), "t1 is not present at shutdown.");
            assert!(!t2.is_present(ctx), "t2 is present at shutdown.");
        }
    }
}

#[test]
fn delay_int() {
    let config = runtime::Config::default()
        .with_fast_forward(true)
        .with_timeout(Duration::seconds(1));
    let (_, _env) =
        boomerang_util::runner::build_and_test_reactor(Main(), "main", (), config).unwrap();
}
