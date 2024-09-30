//! A PCAP file reader Reactor

use std::thread::JoinHandle;

use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};

pub struct State {
    thread: Option<JoinHandle<()>>,
    cap: Option<pcap::Capture<pcap::Offline>>,
    codec: Box<dyn pcap::PacketCodec>,
}

#[derive(Reactor)]
#[reactor(state = "State", reaction = "ReactionStartup")]
struct PcapReader {
    packet: TypedActionKey<(), Physical>,
}

#[derive(Reaction)]
#[reaction(reactor = "PcapReader", triggers(startup))]
struct ReactionStartup {
    packet: runtime::PhysicalActionRef<()>,
}

impl Trigger<PcapReader> for ReactionStartup {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut <PcapReader as Reactor>::State) {
        let mut send_ctx = ctx.make_send_context();
        let mut packet_action = self.packet.clone();
        let mut cap = state.cap.take().unwrap();

        state.thread = Some(std::thread::spawn(move || {

            for item in cap.iter

            while let Ok(packet) = cap.next_packet() {
                send_ctx.send(&mut packet_action, ());
            }
        }));
    }
}
