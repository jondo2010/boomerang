//! Host-only topology declarations composed from owning Cargo crates.

use snake_game::game;
use snake_keyboard::{display, keyboard};

use boomerang::builder::compiler::{ApplicationTopology, TopologyAuthoringError, TopologyBuilder};

pub fn topology() -> Result<ApplicationTopology, TopologyAuthoringError> {
    let mut app = TopologyBuilder::new("application/keyboard/snake")?;
    // Both components share the enclave's shutdown lifecycle.
    let enclave = app.enclave("keyboard")?;
    let keyboard = app.component("keyboard", keyboard::definition(), &enclave)?;
    let snake = app.component("snake", game::definition(), &enclave)?;
    app.connect(&keyboard.key, &snake.key)?;
    // Initialize the terminal before Snake renders and starts its game clock.
    app.connect(&keyboard.ready, &snake.ready)?;
    app.finish()
}

pub fn keyboard_topology() -> Result<ApplicationTopology, TopologyAuthoringError> {
    let mut app = TopologyBuilder::new("application/display/keyboard")?;
    let enclave = app.enclave("keyboard")?;
    let keyboard = app.component("keyboard", keyboard::definition(), &enclave)?;
    let display = app.component("display", display::definition(), &enclave)?;
    app.connect(&keyboard.key, &display.key)?;
    app.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_component_drives_both_example_consumers() {
        for (topology, consumer) in [
            (topology().unwrap(), "snake"),
            (keyboard_topology().unwrap(), "display"),
        ] {
            assert_eq!(topology.components().count(), 2);
            // Both components share an enclave, including its shutdown lifecycle.
            assert_eq!(topology.enclaves().count(), 1);
            let connections: Vec<_> = topology
                .connections()
                .map(|(_, connection)| {
                    (
                        connection.source().to_string(),
                        connection.target().to_string(),
                    )
                })
                .collect();
            assert!(connections.contains(&("keyboard/key".into(), format!("{consumer}/key"))));
            if consumer == "snake" {
                assert!(connections.contains(&("keyboard/ready".into(), "snake/ready".into())));
            }
        }
    }
}
