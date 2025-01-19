#[cfg(not(windows))]
mod support;

#[cfg(not(windows))]
mod keyboard_events;

#[cfg(not(windows))]
mod reactor {
    use super::support::*;
    use crate::keyboard_events::{Key, KeyboardEvents, KeyboardEventsBuilder};
    use boomerang::prelude::*;

    #[derive(Reactor)]
    #[reactor(
        state = "Snake",
        reaction = "ReactionStartup",
        reaction = "ReactionScreenRefresh",
        reaction = "ReactionMoreFood",
        reaction = "ReactionKeyboard",
        reaction = "ReactionAddFood",
        reaction = "ReactionShutdown"
    )]
    pub struct SnakeBuilder {
        /// this thing helps capturing key presses
        #[reactor(child= KeyboardEvents::default())]
        keyboard: KeyboardEventsBuilder,

        /// Triggers a screen refresh, not a timer because we can
        /// shrink the period over time to speed up the game.
        screen_refresh: TypedActionKey,

        /// manually triggered
        #[reactor(action())]
        manually_add_more_food: TypedActionKey,

        /// periodic
        #[reactor(timer(period = "5 sec"))]
        add_more_food: TimerActionKey,
    }

    #[derive(Debug)]
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
                pending_direction: Direction::Right,
                snake_direction: Direction::Right,
                food_on_grid: 0,
                max_food_on_grid: food_limit,
            }
        }
    }

    #[derive(Reaction)]
    #[reaction(reactor = "SnakeBuilder", triggers(startup))]
    struct ReactionStartup<'a> {
        screen_refresh: runtime::ActionRef<'a>,
    }

    impl runtime::Trigger<Snake> for ReactionStartup<'_> {
        fn trigger(
            mut self,
            ctx: &mut runtime::Context,
            state: &mut <SnakeBuilder as Reactor>::State,
        ) {
            // KeyboardEvents makes stdout raw on startup so this is safe
            output::paint_on_raw_console(&state.grid);

            // schedule the first one, then it reschedules itself.
            self.screen_refresh
                .schedule(ctx, (), Some(Duration::milliseconds(1000)));
        }
    }

    #[derive(Reaction)]
    #[reaction(reactor = "SnakeBuilder")]
    struct ReactionScreenRefresh<'a> {
        #[reaction(triggers)]
        screen_refresh: runtime::ActionRef<'a>,
    }

    impl runtime::Trigger<Snake> for ReactionScreenRefresh<'_> {
        fn trigger(
            mut self,
            ctx: &mut runtime::Context,
            state: &mut <SnakeBuilder as Reactor>::State,
        ) {
            // select a delay depending on the tempo
            let delay = Duration::milliseconds(400)
                - (state.tempo_step * state.tempo).min(Duration::milliseconds(300));
            self.screen_refresh.schedule(ctx, (), Some(delay));
        }
    }

    #[derive(Reaction)]
    #[reaction(reactor = "SnakeBuilder", triggers(action = "screen_refresh"))]
    struct ReactionMoreFood<'a> {
        manually_add_more_food: runtime::ActionRef<'a>,
    }

    impl runtime::Trigger<Snake> for ReactionMoreFood<'_> {
        fn trigger(
            mut self,
            ctx: &mut runtime::Context,
            state: &mut <SnakeBuilder as Reactor>::State,
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
                        self.manually_add_more_food.schedule(ctx, (), None);
                    }
                    state.tempo += 1;
                }
                UpdateResult::NothingInParticular => { /* do nothing in particular. */ }
            }

            output::paint_on_raw_console(&state.grid);
        }
    }

    #[derive(Reaction)]
    #[reaction(reactor = "SnakeBuilder")]
    struct ReactionKeyboard<'a> {
        #[reaction(path = "keyboard.arrow_key_pressed")]
        arrow_key_pressed: runtime::InputRef<'a, Key>,
    }

    impl runtime::Trigger<Snake> for ReactionKeyboard<'_> {
        fn trigger(
            self,
            _ctx: &mut runtime::Context,
            state: &mut <SnakeBuilder as Reactor>::State,
        ) {
            // this might be overwritten several times, only committed on screen refreshes
            state.pending_direction = match self.arrow_key_pressed.unwrap() {
                Key::Left => Direction::Left,
                Key::Right => Direction::Right,
                Key::Up => Direction::Up,
                Key::Down => Direction::Down,
                _ => unreachable!(),
            };
        }
    }

    #[derive(Reaction)]
    #[reaction(
        reactor = "SnakeBuilder",
        triggers(action = "manually_add_more_food"),
        triggers(action = "add_more_food")
    )]
    struct ReactionAddFood;

    impl runtime::Trigger<Snake> for ReactionAddFood {
        fn trigger(
            self,
            _ctx: &mut runtime::Context,
            state: &mut <SnakeBuilder as Reactor>::State,
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
    #[reaction(reactor = "SnakeBuilder", triggers(shutdown))]
    struct ReactionShutdown;

    impl runtime::Trigger<Snake> for ReactionShutdown {
        fn trigger(
            self,
            _ctx: &mut runtime::Context,
            state: &mut <SnakeBuilder as Reactor>::State,
        ) {
            println!("Game over! Your score was: {}", state.snake.len());
        }
    }
}

#[cfg(not(windows))]
fn main() {
    use reactor::{Snake, SnakeBuilder};
    let _ = boomerang_util::runner::build_and_run_reactor::<SnakeBuilder>(
        "snake",
        Snake::new(32, boomerang::runtime::Duration::milliseconds(40), 2),
    )
    .unwrap();
}

#[cfg(windows)]
fn main() {}
