use boomerang::builder::compiler::TopologyBuilder;
use snake_game::game;
use snake_keyboard::{display, keyboard};

#[test]
fn public_keyboard_api_composes_without_importing_implementation_modules() {
    let mut app = TopologyBuilder::new("application/consumer/keys").unwrap();
    let enclave = app.enclave("keys").unwrap();
    let keyboard = app
        .component("keys", keyboard::definition(), &enclave)
        .unwrap();
    let display = app
        .component("consumer", display::definition(), &enclave)
        .unwrap();
    app.connect(&keyboard.key, &display.key).unwrap();
    let topology = app.finish().unwrap();
    assert_eq!(topology.components().count(), 2);
    assert_eq!(topology.connections().count(), 1);
    assert_eq!(topology.enclaves().count(), 1);
}

#[test]
fn public_game_api_composes_from_its_own_crate() {
    let mut app = TopologyBuilder::new("application/game").unwrap();
    let enclave = app.enclave("game").unwrap();
    app.component("game", game::definition(), &enclave).unwrap();
    assert_eq!(app.finish().unwrap().components().count(), 1);
}
