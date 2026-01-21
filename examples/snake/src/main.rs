mod keyboard_events;
mod support;

use boomerang::prelude::*;

use crossterm::event::KeyCode;
use keyboard_events::{KeyboardEvents, KeyboardEventsState};

use support::*;

pub struct SnakeState {
    // model classes for the game.
    snake: CircularSnake,
    grid: SnakeGrid, // note that this one borrows snake temporarily

    /// The game speed level
    tempo: u32,
    tempo_step: Duration,

    /// Changes with arrow key presses, might be invalid.
    /// Only committed to snake_direction on grid update.
    pending_direction: Direction,
    /// Whither the snake has slithered last
    snake_direction: Direction,

    // state vars for food
    food_on_grid: u32,
    max_food_on_grid: u32,
}

impl SnakeState {
    pub fn new(grid_side: usize, tempo_step: Duration, food_limit: u32) -> Self {
        let snake = CircularSnake::new(grid_side);
        let grid = SnakeGrid::new(grid_side, &snake);
        Self {
            snake,
            grid,
            tempo: 1,
            tempo_step,
            pending_direction: Direction::Right,
            snake_direction: Direction::Right,
            food_on_grid: 0,
            max_food_on_grid: food_limit,
        }
    }
}

#[reactor(state = SnakeState)]
fn Snake() -> impl Reactor {
    // this thing helps capturing key presses
    let keyboard = builder.add_child_reactor(
        KeyboardEvents(),
        "keyboard",
        KeyboardEventsState::default(),
        false,
    )?;

    // Triggers a screen refresh, not a timer because we can shrink the period over time to speed up the game.
    let screen_refresh = builder.add_logical_action("screen_refresh", None)?;

    // manually triggered
    let manually_add_more_food = builder.add_logical_action("manually_add_more_food", None)?;

    // periodic
    let add_more_food = builder.add_timer(
        "add_more_food",
        TimerSpec::default().with_period(Duration::seconds(5)),
    )?;

    builder
        .add_reaction(Some("Startup"))
        .with_startup_trigger()
        .with_effect(screen_refresh)
        .with_reaction_fn(|_ctx, state, (_, mut screen_refresh)| {
            // KeyboardEvents makes stdout raw on startup so this is safe
            output::paint_on_raw_console(&state.grid);

            // schedule the first one, then it reschedules itself.
            _ctx.schedule_action(&mut screen_refresh, (), Some(Duration::seconds(1)));
        })
        .finish()?;

    builder
        .add_reaction(Some("ScreenRefresh"))
        .with_trigger(screen_refresh)
        .with_reaction_fn(|_ctx, state, (mut screen_refresh,)| {
            // select a delay depending on the tempo
            let delay = Duration::milliseconds(400)
                - (state.tempo_step * state.tempo).min(Duration::milliseconds(300));
            _ctx.schedule_action(&mut screen_refresh, (), Some(delay));
        })
        .finish()?;

    builder
        .add_reaction(Some("MoreFood"))
        .with_trigger(screen_refresh)
        .with_effect(manually_add_more_food)
        .with_reaction_fn(|ctx, state, (_, mut manually_add_more_food)| {
            // check that the user's command is valid
            if state.pending_direction != state.snake_direction.opposite() {
                state.snake_direction = state.pending_direction;
            }

            match state
                .snake
                .slither_forward(state.snake_direction, &mut state.grid)
            {
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
                UpdateResult::NothingInParticular => { /* do nothing in particular. */ }
            }

            output::paint_on_raw_console(&state.grid);
        })
        .finish()?;

    builder
        .add_reaction(Some("Keyboard"))
        .with_trigger(keyboard.arrow_key_pressed)
        .with_reaction_fn(|_ctx, state, (key_event,)| {
            // this might be overwritten several times, only committed on screen refreshes
            state.pending_direction = match key_event.as_ref().map(|k| k.code) {
                Some(KeyCode::Left) => Direction::Left,
                Some(KeyCode::Right) => Direction::Right,
                Some(KeyCode::Up) => Direction::Up,
                Some(KeyCode::Down) => Direction::Down,
                _ => unreachable!(),
            };
        })
        .finish()?;

    builder
        .add_reaction(Some("AddFood"))
        .with_trigger(manually_add_more_food)
        .with_trigger(add_more_food)
        .with_reaction_fn(|_ctx, state, (_, _)| {
            if state.food_on_grid >= state.max_food_on_grid {
                return; // there's enough food there
            }

            if let Some(cell) = state.grid.find_random_free_cell() {
                state.grid[cell] = CellState::Food; // next screen update will catch this.
                state.food_on_grid += 1;
            }
        })
        .finish()?;

    builder
        .add_reaction(Some("Shutdown"))
        .with_shutdown_trigger()
        .with_reaction_fn(|_ctx, state, (_,)| {
            println!("Game over! Your score was: {}", state.snake.len());
        })
        .finish()?;
}

fn main() {
    let _ = boomerang_util::runner::build_and_run_reactor(
        Snake(),
        "snake",
        SnakeState::new(16, boomerang::runtime::Duration::milliseconds(40), 2),
    )
    .unwrap();
}
