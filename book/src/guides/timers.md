# Timers and Delays

Timers are the most common trigger. Delays are applied on connections to shift events in logical time.

```rust
use boomerang::prelude::*;

#[reactor]
fn Source(#[output] out: u32) -> impl Reactor {
    timer! { t(0 s, 100 msec) };
    reaction! {
        (t) -> out {
            *out = Some(1);
        }
    }
}

#[reactor]
fn Sink(#[input] inp: u32) -> impl Reactor {
    reaction! {
        (inp) {
            println!("got {:?}", *inp);
        }
    }
}

#[reactor]
fn Top() -> impl Reactor {
    let source = builder.add_child_reactor(Source(), "source", (), false)?;
    let sink = builder.add_child_reactor(Sink(), "sink", (), false)?;
    builder.connect_port(source.out, sink.inp, Some(Duration::milliseconds(10)), false)?;
}
```

Delays are expressed as `Duration` and applied to the connection. The delayed event will appear at a later tag.
