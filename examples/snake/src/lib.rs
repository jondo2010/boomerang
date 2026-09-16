//! Host-only topology declarations composed from owning Cargo crates.

use snake_game::game;
use snake_keyboard::{display, keyboard};

use boomerang::builder::compiler::{ApplicationTopology, TopologyBuildError};
use boomerang::prelude::*;

pub fn topology() -> Result<ApplicationTopology, TopologyBuildError> {
    let mut assembly = Assembly::new();
    let keyboard = add_keyboard(&mut assembly);
    let snake = game::Snake()
        .build(
            "snake",
            game::SnakeState::default(),
            None,
            None,
            None,
            false,
            &mut assembly,
        )
        .expect("valid Snake reactor declaration");
    assembly
        .add_port_connection::<crossterm::event::KeyEvent, _, _>(
            keyboard.key,
            snake.key,
            None,
            false,
        )
        .expect("keyboard drives Snake");
    assembly
        .add_port_connection::<(), _, _>(keyboard.ready, snake.ready, None, false)
        .expect("keyboard initializes the terminal before Snake renders");
    Ok(assembly
        .application_topology()
        .expect("valid Snake composition"))
}

pub fn keyboard_topology() -> Result<ApplicationTopology, TopologyBuildError> {
    let mut assembly = Assembly::new();
    let keyboard = add_keyboard(&mut assembly);
    let display = display::ArrowDisplay()
        .build("display", (), None, None, None, false, &mut assembly)
        .expect("valid arrow display declaration");
    assembly
        .add_port_connection::<crossterm::event::KeyEvent, _, _>(
            keyboard.key,
            display.key,
            None,
            false,
        )
        .expect("keyboard drives the arrow display");
    Ok(assembly
        .application_topology()
        .expect("valid keyboard composition"))
}

fn add_keyboard(assembly: &mut Assembly) -> keyboard::KeyboardPorts {
    // Local roots share one enclave, so Ctrl-C and game-over shut down both components.
    keyboard::Keyboard()
        .build(
            "keyboard",
            keyboard::KeyboardState::default(),
            None,
            None,
            None,
            false,
            assembly,
        )
        .expect("valid keyboard reactor declaration")
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
