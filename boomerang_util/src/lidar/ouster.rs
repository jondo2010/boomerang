use std::{path::Path, thread::JoinHandle};

use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};

use lidar_utils::ouster;

const UDP_HEADER_SIZE: usize = 42;

fn ouster_pcd_converter() -> anyhow::Result<()> {
    cap.filter("udp", true)?;

    while let Ok(packet) = cap.next() {
        let slice = &packet.data[UDP_HEADER_SIZE..];
        let lidar_packet = ouster::Packet::from_slice(slice)?;
        let points = pcd_converter.convert(lidar_packet)?;
        assert!(points.len() as u16 == pcd_converter.columns_per_revolution());
    }

    Ok(())
}

pub struct State {
    thread: Option<JoinHandle<()>>,
    config_path: &'static str,
    pcap_path: &'static str,
}

#[derive(Reactor)]
#[reactor(state = "State", reaction = "ReactionStartup")]
pub struct OusterLidar {
    packet: TypedActionKey<(), Physical>,
}

#[derive(Reaction)]
#[reaction(reactor = "OusterLidar", triggers(startup))]
struct ReactionStartup {
    packet: runtime::PhysicalActionRef<()>,
}

impl Trigger<OusterLidar> for ReactionStartup {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut <OusterLidar as Reactor>::State) {
        let mut send_ctx = ctx.make_send_context();
        let mut packet = self.packet.clone();

        // Load config
        let config = ouster::Config::from_path(&state.config_path).unwrap();
        let pcd_converter = ouster::PointCloudConverter::from_config(config);

        // Load pcap file
        let mut cap = pcap::Capture::from_file("test_files/ouster_example.pcap")?;

        state.thread = Some(std::thread::spawn(move || {}));
    }
}
