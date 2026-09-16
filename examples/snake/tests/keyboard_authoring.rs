use boomerang::prelude::*;
use snake_game::game::{Snake, SnakeState};
use snake_keyboard::{
    display::ArrowDisplay,
    keyboard::{Keyboard, KeyboardState},
};

#[test]
fn public_keyboard_api_composes_without_importing_implementation_modules() {
    let mut assembly = Assembly::new();
    let keyboard = Keyboard()
        .build(
            "keys",
            KeyboardState::default(),
            None,
            None,
            None,
            false,
            &mut assembly,
        )
        .unwrap();
    let display = ArrowDisplay()
        .build("consumer", (), None, None, None, false, &mut assembly)
        .unwrap();
    assembly
        .add_port_connection::<crossterm::event::KeyEvent, _, _>(
            keyboard.key,
            display.key,
            None,
            false,
        )
        .unwrap();
    let topology = assembly.application_topology().unwrap();
    assert_eq!(topology.components().count(), 2);
    assert_eq!(topology.connections().count(), 1);
    assert_eq!(topology.enclaves().count(), 1);
}

#[test]
fn public_game_api_composes_from_its_own_crate() {
    let mut assembly = Assembly::new();
    Snake()
        .build(
            "game",
            SnakeState::default(),
            None,
            None,
            None,
            false,
            &mut assembly,
        )
        .unwrap();
    assert_eq!(
        assembly
            .application_topology()
            .unwrap()
            .components()
            .count(),
        1
    );
}
