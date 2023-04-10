#![cfg(not(windows))]

//! A snake terminal game. Does not support windows.
//!
//! Original source:
//!     Author: Clément Fournier
//!     Git: https://github.com/lf-lang/reactor-rust/examples/src/Snake.lf

// Panic on windows, since we don't support that platform
#[cfg(windows)]
compile_error!("This example does not support Windows");

use boomerang::{
    builder::{BuilderReactionKey, TypedActionKey},
    run, runtime, Reactor,
};
use boomerang_util::keyboard_events::{Key, KeyboardEvents, KeyboardEventsBuilder};

mod support;

#[derive(Reactor)]
#[reactor(state = "Snake")]
struct SnakeBuilder {
    /// this thing helps capturing key presses
    #[reactor(child(state = "KeyboardEvents::new()"))]
    keyboard: KeyboardEventsBuilder,

    /// Triggers a screen refresh, not a timer because we can
    /// shrink the period over time to speed up the game.
    #[reactor(action(physical = "false"))]
    screen_refresh: TypedActionKey,

    /// manually triggered
    #[reactor(action())]
    manually_add_more_food: TypedActionKey,

    /// periodic
    #[reactor(timer(period = "5 sec"))]
    add_more_food: TypedActionKey,

    #[reactor(reaction(function = "Snake::reaction_startup"))]
    reaction_startup: BuilderReactionKey,

    #[reactor(reaction(function = "Snake::reaction_screen_refresh"))]
    reaction_screen_refresh: BuilderReactionKey,

    #[reactor(reaction(function = "Snake::reaction_more_food"))]
    reaction_more_food: BuilderReactionKey,

    #[reactor(reaction(function = "Snake::reaction_keyboard"))]
    reaction_keyboard: BuilderReactionKey,

    #[reactor(reaction(function = "Snake::reaction_add_food"))]
    reaction_add_food: BuilderReactionKey,

    #[reactor(reaction(function = "Snake::reaction_shutdown"))]
    reaction_shutdown: BuilderReactionKey,
}

struct Snake {
    // model classes for the game.
    snake: support::CircularSnake,
    grid: support::SnakeGrid, // note that this one borrows snake temporarily

    /// The game speed level
    tempo: u32,
    tempo_step: runtime::Duration,

    /// Changes with arrow key presses, might be invalid.
    /// Only committed to snake_direction on grid update.
    pending_direction: support::Direction,
    /// Whither the snake has slithered last
    snake_direction: support::Direction,

    // state vars for food
    food_on_grid: u32,
    max_food_on_grid: u32,
}

impl Snake {
    fn new(grid_side: usize, tempo_step: runtime::Duration, food_limit: u32) -> Self {
        let snake = support::CircularSnake::new(grid_side);
        let grid = support::SnakeGrid::new(grid_side, &snake);
        Self {
            snake,
            grid,
            tempo: 1,
            tempo_step,
            pending_direction: support::Direction::RIGHT,
            snake_direction: support::Direction::RIGHT,
            food_on_grid: 0,
            max_food_on_grid: food_limit,
        }
    }

    // @label startup
    #[boomerang::reaction(reactor = "SnakeBuilder", triggers(startup))]
    fn reaction_startup(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(effects)] mut screen_refresh: runtime::ActionRef,
    ) {
        // KeyboardEvents makes stdout raw on startup so this is safe
        support::output::paint_on_raw_console(&self.grid);

        // schedule the first one, then it reschedules itself.
        ctx.schedule_action(
            &mut screen_refresh,
            None,
            Some(runtime::Duration::from_secs(1)),
        )
    }

    // @label schedule_next_tick
    #[boomerang::reaction(reactor = "SnakeBuilder")]
    fn reaction_screen_refresh(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(triggers, effects)] mut screen_refresh: runtime::ActionRef,
    ) {
        // select a delay depending on the tempo
        let delay = runtime::Duration::from_millis(400)
            - (self.tempo_step * self.tempo).min(runtime::Duration::from_millis(300));
        ctx.schedule_action(&mut screen_refresh, None, Some(delay));
    }

    // @label refresh_screen
    #[boomerang::reaction(reactor = "SnakeBuilder", triggers(action = "screen_refresh"))]
    fn reaction_more_food(
        &mut self,
        ctx: &mut runtime::Context,
        #[reactor::action(effects)] mut manually_add_more_food: runtime::ActionRef,
    ) {
        // check that the user's command is valid
        if self.pending_direction != self.snake_direction.opposite() {
            self.snake_direction = self.pending_direction;
        }

        match self
            .snake
            .slither_forward(self.snake_direction, &mut self.grid)
        {
            support::UpdateResult::GameOver => {
                ctx.schedule_shutdown(None);
                return;
            }
            support::UpdateResult::FoodEaten => {
                self.food_on_grid -= 1;
                if self.food_on_grid == 0 {
                    ctx.schedule_action(&mut manually_add_more_food, None, None);
                }
                self.tempo += 1;
            }
            support::UpdateResult::NothingInParticular => { /* do nothing in particular. */ }
        }

        support::output::paint_on_raw_console(&self.grid);
    }

    // @label handle_key_press
    #[boomerang::reaction(reactor = "SnakeBuilder")]
    fn reaction_keyboard(
        &mut self,
        _ctx: &mut runtime::Context,
        #[reactor::port(triggers, path = "keyboard.arrow_key_pressed")]
        arrow_key_pressed: &runtime::Port<Key>,
    ) {
        // this might be overwritten several times, only committed on screen refreshes
        self.pending_direction = match arrow_key_pressed.get().unwrap() {
            Key::Left => support::Direction::LEFT,
            Key::Right => support::Direction::RIGHT,
            Key::Up => support::Direction::UP,
            Key::Down => support::Direction::DOWN,
            _ => unreachable!(),
        };
    }

    // @label add_food
    #[boomerang::reaction(
        reactor = "SnakeBuilder",
        triggers(action = "manually_add_more_food", action = "add_more_food")
    )]
    fn reaction_add_food(&mut self, _ctx: &mut runtime::Context) {
        if self.food_on_grid >= self.max_food_on_grid {
            return; // there's enough food there
        }

        if let Some(cell) = self.grid.find_random_free_cell() {
            self.grid[cell] = support::CellState::Food; // next screen update will catch this.
            self.food_on_grid += 1;
        }
    }

    // @label shutdown
    #[boomerang::reaction(reactor = "SnakeBuilder", triggers(shutdown))]
    fn reaction_shutdown(&mut self, _ctx: &mut runtime::Context) {
        println!("Game over! Your score was: {}", self.snake.len());
    }
}

fn main() {
    let _ = run::build_and_run_reactor::<SnakeBuilder>(
        "multiple_contained",
        Snake::new(32, runtime::Duration::from_millis(40), 2),
    )
    .unwrap();
}
