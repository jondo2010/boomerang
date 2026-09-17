//! Owning Cargo crate for the Snake game component.

boomerang::component! {
pub mod game {

mod support;

use boomerang::prelude::*;
use crossterm::event::{KeyCode, KeyEvent};
use support::*;

pub struct SnakeState {
    snake: CircularSnake,
    grid: SnakeGrid,
    tempo: u32,
    tempo_step: Duration,
    pending_direction: Direction,
    snake_direction: Direction,
    food_on_grid: u32,
    max_food_on_grid: u32,
}

impl Default for SnakeState {
    fn default() -> Self {
        let snake = CircularSnake::new(16);
        let grid = SnakeGrid::new(16, &snake);
        Self {
            snake,
            grid,
            tempo: 1,
            tempo_step: Duration::milliseconds(40),
            pending_direction: Direction::Right,
            snake_direction: Direction::Right,
            food_on_grid: 0,
            max_food_on_grid: 2,
        }
    }
}

#[reactor(
    state = SnakeState, state_init = SnakeState::default,
    contract = "snake.game", contract_version = 1,
    bounds(queue_capacity = 1024, payload_bytes = 32768, state_bytes = 16384, scratch_bytes = 16384)
)]
pub fn Snake(
    #[input] key: KeyEvent,
    #[input] ready: (),
    #[logical_action] screen_refresh: (),
    #[logical_action] manually_add_more_food: (),
    #[logical_action] add_more_food: (),
) -> impl Reactor {
    reaction! {
        Startup (ready) -> screen_refresh, add_more_food {
            // The keyboard component emits ready only after enabling raw mode.
            output::paint_on_raw_console(&state.grid);
            ctx.schedule_action(&mut screen_refresh, (), Some(Duration::seconds(1)));
            ctx.schedule_action(&mut add_more_food, (), None);
        }
    }
    reaction! {
        ScreenRefresh (screen_refresh) {
            let delay = Duration::milliseconds(400)
                - (state.tempo_step * state.tempo).min(Duration::milliseconds(300));
            ctx.schedule_action(&mut screen_refresh, (), Some(delay));
        }
    }
    reaction! {
        Move (screen_refresh) -> manually_add_more_food {
            if state.pending_direction != state.snake_direction.opposite() {
                state.snake_direction = state.pending_direction;
            }
            match state.snake.slither_forward(state.snake_direction, &mut state.grid) {
                UpdateResult::GameOver => {
                    ctx.schedule_shutdown(None);
                    return;
                }
                UpdateResult::FoodEaten => {
                    state.food_on_grid -= 1;
                    if state.food_on_grid == 0 {
                        ctx.schedule_action(&mut manually_add_more_food, (), None);
                    }
                    state.tempo += 1;
                }
                UpdateResult::NothingInParticular => {}
            }
            output::paint_on_raw_console(&state.grid);
        }
    }
    reaction! {
        Keyboard (key) {
            state.pending_direction = match key.as_ref().map(|event| event.code) {
                Some(KeyCode::Left) => Direction::Left,
                Some(KeyCode::Right) => Direction::Right,
                Some(KeyCode::Up) => Direction::Up,
                Some(KeyCode::Down) => Direction::Down,
                _ => return,
            };
        }
    }
    reaction! {
        FoodClock (add_more_food) {
            // Compiled execution uses a rescheduling action for the periodic timer.
            ctx.schedule_action(&mut add_more_food, (), Some(Duration::seconds(5)));
        }
    }
    reaction! {
        AddFood (manually_add_more_food, add_more_food) {
            if state.food_on_grid >= state.max_food_on_grid {
                return;
            }
            if let Some(cell) = state.grid.find_random_free_cell() {
                state.grid[cell] = CellState::Food;
                state.food_on_grid += 1;
            }
        }
    }
    reaction! {
        Shutdown (shutdown) {
            // Shutdown reactions may run before the keyboard restores cooked mode.
            print!("Game over! Your score was: {}\r\n", state.snake.len());
        }
    }
}
}
}
