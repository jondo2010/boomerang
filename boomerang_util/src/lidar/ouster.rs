use std::thread::JoinHandle;

use boomerang::prelude::*;

use lidar_utils::ouster::{self};

const UDP_HEADER_SIZE: usize = 42;

pub struct State {
    thread: Option<JoinHandle<()>>,
    pcd_cap: Option<(ouster::PointCloudConverter, pcap::Capture<pcap::Offline>)>,
    first_ts: Option<std::time::Duration>,
}

impl State {
    fn new_offline(config_path: &'static str, pcap_path: &'static str) -> anyhow::Result<Self> {
        // Load config
        let config = ouster::Config::from_path(config_path)?;
        let pcd_converter = ouster::PointCloudConverter::from_config(config);

        // Load pcap file
        let mut cap = pcap::Capture::from_file(pcap_path)?;
        cap.filter("udp", true)?;

        Ok(Self {
            thread: None,
            pcd_cap: Some((pcd_converter, cap)),
            first_ts: None,
        })
    }
}

#[derive(Reactor)]
#[reactor(state = "State", reaction = "ReactionStartup<N>")]
pub struct OusterLidar<const N: usize> {
    packet: TypedActionKey<[ouster::Point; N], Physical>,
}

#[derive(Reaction)]
#[reaction(reactor = "OusterLidar<N>", triggers(startup))]
struct ReactionStartup<const N: usize> {
    packet: runtime::PhysicalActionRef<[ouster::Point; N]>,
}

impl<const N: usize> Trigger<OusterLidar<N>> for ReactionStartup<N> {
    fn trigger(self, ctx: &mut runtime::Context, state: &mut <OusterLidar<N> as Reactor>::State) {
        let mut send_ctx = ctx.make_send_context();
        let mut packet_action = self.packet.clone();
        let (pcd_converter, mut cap) = state.pcd_cap.take().unwrap();

        state.thread = Some(std::thread::spawn(move || {
            let mut first_ts = None;

            while let Ok(packet) = cap.next_packet() {
                let packet_ts = std::time::Duration::new(
                    packet.header.ts.tv_sec as u64,
                    packet.header.ts.tv_usec as u32 * 1000,
                );

                let delay = packet_ts
                    .checked_sub(*first_ts.get_or_insert_with(|| packet_ts))
                    .inspect(|delay| {
                        tracing::info!("Delay: {:?}", delay);
                    })
                    .and_then(|delay| delay.checked_sub(send_ctx.get_elapsed_physical_time()));
                if delay.is_none() {
                    eprintln!("Timestamps are not monotonically increasing");
                }

                let slice = &packet.data[UDP_HEADER_SIZE..];
                let lidar_packet = ouster::Packet::from_slice(slice).unwrap();
                let points = pcd_converter.convert(lidar_packet).unwrap();
                assert!(points.len() as u16 == pcd_converter.columns_per_revolution());
                let points: [ouster::Point; N] = points.try_into().unwrap();
                send_ctx.schedule_action(&mut packet_action, Some(points), delay);
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ouster() {
        tracing_subscriber::fmt::init();
        let state = State::new_offline("../ouster_example.json", "../ouster_example.pcap").unwrap();
        let config = runtime::Config::default()
            .with_keep_alive(true)
            .with_queue_size(5);
        let _ = crate::runner::build_and_test_reactor::<OusterLidar<1024>>("lidar", state, config);
    }
}
