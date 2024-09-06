//! A snake terminal game. Does not support windows.
//!
//! Original source:
//!     Author: Clément Fournier
//!     Git: https://github.com/lf-lang/reactor-rust/examples/src/Snake.lf

#[cfg(not(windows))]
mod support;

#[cfg(not(windows))]
mod reactor {
    use super::support::*;
    use boomerang::{builder::prelude::*, runtime, Reaction, Reactor};
    use boomerang_util::keyboard_events::{Key, KeyboardEvents, KeyboardEventsBuilder};

    #[derive(Clone, Reactor)]
    #[reactor(state = Snake)]
    pub struct SnakeBuilder {
        /// this thing helps capturing key presses
        #[reactor(child= KeyboardEvents::default())]
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
        add_more_food: TimerActionKey,

        reaction_startup: TypedReactionKey<ReactionStartup<'static>>,
        reaction_screen_refresh: TypedReactionKey<ReactionScreenRefresh<'static>>,
        reaction_more_food: TypedReactionKey<ReactionMoreFood<'static>>,
        reaction_keyboard: TypedReactionKey<ReactionKeyboard<'static>>,
        reaction_add_food: TypedReactionKey<ReactionAddFood>,
        reaction_shutdown: TypedReactionKey<ReactionShutdown>,
    }

    pub struct Snake {
        // model classes for the game.
        snake: CircularSnake,
        grid: SnakeGrid, // note that this one borrows snake temporarily

        /// The game speed level
        tempo: u32,
        tempo_step: runtime::Duration,

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
        pub fn new(grid_side: usize, tempo_step: runtime::Duration, food_limit: u32) -> Self {
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
    }

    #[derive(Reaction)]
    #[reaction(triggers(startup))]
    struct ReactionStartup<'a> {
        screen_refresh: runtime::ActionRef<'a>,
    }

    impl Trigger for ReactionStartup<'_> {
        type Reactor = SnakeBuilder;
        fn trigger(
            &mut self,
            ctx: &mut runtime::Context,
            state: &mut <Self::Reactor as Reactor>::State,
        ) {
            // KeyboardEvents makes stdout raw on startup so this is safe
            output::paint_on_raw_console(&state.grid);

            // schedule the first one, then it reschedules itself.
            ctx.schedule_action(
                &mut self.screen_refresh,
                None,
                Some(runtime::Duration::from_millis(1000)),
            );
        }
    }

    #[derive(Reaction)]
    struct ReactionScreenRefresh<'a> {
        #[reaction(triggers)]
        screen_refresh: runtime::ActionRef<'a>,
    }

    impl Trigger for ReactionScreenRefresh<'_> {
        type Reactor = SnakeBuilder;
        fn trigger(
            &mut self,
            ctx: &mut runtime::Context,
            state: &mut <Self::Reactor as Reactor>::State,
        ) {
            // select a delay depending on the tempo
            let delay = runtime::Duration::from_millis(400)
                - (state.tempo_step * state.tempo).min(runtime::Duration::from_millis(300));
            ctx.schedule_action(&mut self.screen_refresh, None, Some(delay));
        }
    }

    #[derive(Reaction)]
    #[reaction(triggers(action = "screen_refresh"))]
    struct ReactionMoreFood<'a> {
        manually_add_more_food: runtime::ActionRef<'a>,
    }

    impl Trigger for ReactionMoreFood<'_> {
        type Reactor = SnakeBuilder;
        fn trigger(
            &mut self,
            ctx: &mut runtime::Context,
            state: &mut <Self::Reactor as Reactor>::State,
        ) {
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
                        ctx.schedule_action(&mut self.manually_add_more_food, None, None);
                    }
                    state.tempo += 1;
                }
                UpdateResult::NothingInParticular => { /* do nothing in particular. */ }
            }

            output::paint_on_raw_console(&state.grid);
        }
    }

    #[derive(Reaction)]
    struct ReactionKeyboard<'a> {
        #[reaction(path = "keyboard.arrow_key_pressed")]
        arrow_key_pressed: &'a runtime::Port<Key>,
    }

    impl Trigger for ReactionKeyboard<'_> {
        type Reactor = SnakeBuilder;
        fn trigger(
            &mut self,
            _ctx: &mut runtime::Context,
            state: &mut <Self::Reactor as Reactor>::State,
        ) {
            // this might be overwritten several times, only committed on screen refreshes
            state.pending_direction = match self.arrow_key_pressed.get().unwrap() {
                Key::Left => Direction::LEFT,
                Key::Right => Direction::RIGHT,
                Key::Up => Direction::UP,
                Key::Down => Direction::DOWN,
                _ => unreachable!(),
            };
        }
    }

    #[derive(Reaction)]
    #[reaction(
        triggers(action = "manually_add_more_food"),
        triggers(action = "add_more_food")
    )]
    struct ReactionAddFood;

    impl Trigger for ReactionAddFood {
        type Reactor = SnakeBuilder;
        fn trigger(
            &mut self,
            _ctx: &mut runtime::Context,
            state: &mut <Self::Reactor as Reactor>::State,
        ) {
            if state.food_on_grid >= state.max_food_on_grid {
                return; // there's enough food there
            }

            if let Some(cell) = state.grid.find_random_free_cell() {
                state.grid[cell] = CellState::Food; // next screen update will catch this.
                state.food_on_grid += 1;
            }
        }
    }

    #[derive(Reaction)]
    #[reaction(triggers(shutdown))]
    struct ReactionShutdown;

    impl Trigger for ReactionShutdown {
        type Reactor = SnakeBuilder;
        fn trigger(
            &mut self,
            _ctx: &mut runtime::Context,
            state: &mut <Self::Reactor as Reactor>::State,
        ) {
            println!("Game over! Your score was: {}", state.snake.len());
        }
    }
}

#[cfg(not(windows))]
fn main() {
    use reactor::{Snake, SnakeBuilder};
    let _ = boomerang_util::run::build_and_run_reactor::<SnakeBuilder>(
        "snake",
        Snake::new(32, boomerang::runtime::Duration::from_millis(40), 2),
    )
    .unwrap();
}

#[cfg(windows)]
fn main() {}
