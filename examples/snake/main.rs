//! A snake terminal game. Does not support windows.
//!
//! Original source:
//!     Author: Clément Fournier
//!     Git: https://github.com/lf-lang/reactor-rust/examples/src/Snake.lf

#[cfg(not(windows))]
mod support;

#[cfg(not(windows))]
mod reactor {
    use std::time::Duration;

    use super::support::*;
    use boomerang::{
        builder::{BuilderReactionKey, TypedActionKey},
        runtime, Reactor,
    };
    use boomerang_util::keyboard_events::{Key, KeyboardEvents, KeyboardEventsBuilder};

    #[derive(Reactor)]
    #[reactor(state = "Snake")]
    pub struct SnakeBuilder {
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

    #[derive(Clone)]
    pub struct Snake {
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

    impl Snake {
        pub fn new(grid_side: usize, tempo_step: Duration, food_limit: u32) -> Self {
            let snake = CircularSnake::new(grid_side);
            let grid = SnakeGrid::new(grid_side, &snake);
            Self {
                snake,
                grid,
                tempo: 1,
                tempo_step,
                pending_direction: Direction::RIGHT,
                snake_direction: Direction::RIGHT,
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
            output::paint_on_raw_console(&self.grid);

            // schedule the first one, then it reschedules itself.
            ctx.schedule_action(&mut screen_refresh, None, Some(Duration::from_secs(1)))
        }

        // @label schedule_next_tick
        #[boomerang::reaction(reactor = "SnakeBuilder")]
        fn reaction_screen_refresh(
            &mut self,
            ctx: &mut runtime::Context,
            #[reactor::action(triggers, effects)] mut screen_refresh: runtime::ActionRef,
        ) {
            // select a delay depending on the tempo
            let delay = Duration::from_millis(400)
                - (self.tempo_step * self.tempo).min(Duration::from_millis(300));
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
                UpdateResult::GameOver => {
                    ctx.schedule_shutdown(None);
                    return;
                }
                UpdateResult::FoodEaten => {
                    self.food_on_grid -= 1;
                    if self.food_on_grid == 0 {
                        ctx.schedule_action(&mut manually_add_more_food, None, None);
                    }
                    self.tempo += 1;
                }
                UpdateResult::NothingInParticular => { /* do nothing in particular. */ }
            }

            output::paint_on_raw_console(&self.grid);
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
                Key::Left => Direction::LEFT,
                Key::Right => Direction::RIGHT,
                Key::Up => Direction::UP,
                Key::Down => Direction::DOWN,
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
                self.grid[cell] = CellState::Food; // next screen update will catch this.
                self.food_on_grid += 1;
            }
        }

        // @label shutdown
        #[boomerang::reaction(reactor = "SnakeBuilder", triggers(shutdown))]
        fn reaction_shutdown(&mut self, _ctx: &mut runtime::Context) {
            println!("Game over! Your score was: {}", self.snake.len());
        }
    }
}

#[cfg(not(windows))]
fn main() {
    use std::time::Duration;

    use reactor::{Snake, SnakeBuilder};
    let _ = boomerang::runner::build_and_run_reactor::<SnakeBuilder>(
        "multiple_contained",
        Snake::new(32, Duration::from_millis(40), 2),
    )
    .unwrap();
}

#[cfg(windows)]
fn main() {}
