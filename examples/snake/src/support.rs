//! Support module for the snake game example.

use std::ops::{Index, IndexMut};

/// Position on the grid
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Cell {
    row: usize,
    col: usize,
}

pub fn cell(row: usize, col: usize) -> Cell {
    Cell { row, col }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Direction {
    Up,
    Right,
    Down,
    Left,
}

impl Direction {
    pub fn opposite(&self) -> Self {
        match *self {
            Self::Up => Self::Down,
            Self::Down => Self::Up,
            Self::Right => Self::Left,
            Self::Left => Self::Right,
        }
    }
}

impl Cell {
    fn wrap_add(a: usize, add: bool, grid_side: usize) -> usize {
        if a == grid_side - 1 && add {
            0
        } else if a == 0 && !add {
            grid_side - 1
        } else if add {
            a + 1
        } else {
            a - 1
        }
    }

    /// Returns the neighbor cell that's in the given direction,
    /// wrap around the grid if needed.
    fn shift(self, direction: Direction, grid_side: usize) -> Cell {
        let Cell { row, col } = self;
        match direction {
            Direction::Up => Cell {
                row: Self::wrap_add(row, false, grid_side),
                col,
            },
            Direction::Right => Cell {
                row,
                col: Self::wrap_add(col, true, grid_side),
            },
            Direction::Down => Cell {
                row: Self::wrap_add(row, true, grid_side),
                col,
            },
            Direction::Left => Cell {
                row,
                col: Self::wrap_add(col, false, grid_side),
            },
        }
    }
}

/// Keeps track of the positions of the snake, can update them
/// incrementally.
#[derive(Debug)]
pub struct CircularSnake {
    /// Circular buffer, the head field is the first element,
    /// then they're in order when you read from right to left.
    /// ```no_compile
    ///    [3, 2, 1]
    ///           ^head
    /// ```
    /// This makes advancing while preserving order a constant
    /// time/space operation. But growing the snake is linear.
    snake_positions: Vec<Cell>,
    /// Index of the snake head in the position list
    head: usize,
    /// Side of the square grid
    grid_side: usize,
}

impl CircularSnake {
    pub fn new(grid_side: usize) -> Self {
        let mid = grid_side / 2;
        Self {
            snake_positions: vec![cell(mid, mid), cell(mid, mid - 1)],
            head: 0,
            grid_side,
        }
    }

    pub fn head(&self) -> Cell {
        self.snake_positions[self.head]
    }

    pub fn len(&self) -> usize {
        self.snake_positions.len()
    }

    /// Mutate internal state of the grid too.
    /// Returns cells that have changed. If none, the move was illegal
    pub fn slither_forward(
        &mut self,
        snake_heading: Direction,
        grid: &mut SnakeGrid,
    ) -> UpdateResult {
        let old_head = self.snake_positions[self.head];
        let new_head = old_head.shift(snake_heading, self.grid_side);
        match grid[new_head] {
            // we're eating our tail, move is illegal
            CellState::Snake | CellState::SnakeHead => UpdateResult::GameOver,
            CellState::Food => {
                // then the tail is not moving, we increase size of the snake
                self.snake_positions.insert(self.head + 1, new_head);
                self.head += 1;
                grid[old_head] = CellState::Snake;
                grid[new_head] = CellState::SnakeHead;
                UpdateResult::FoodEaten
            }
            CellState::Free => {
                // replace old tail with new head, shift head ptr wrapping around
                let tail = (self.head + 1) % self.snake_positions.len();
                let old_tail = self.snake_positions[tail];
                self.snake_positions[tail] = new_head;
                self.head = tail;

                // note old_head and old_tail may be same if snake has length 1
                grid[old_head] = CellState::Snake;
                grid[old_tail] = CellState::Free;
                grid[new_head] = CellState::SnakeHead;
                UpdateResult::NothingInParticular
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum UpdateResult {
    FoodEaten,
    GameOver,
    NothingInParticular,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CellState {
    SnakeHead,
    Snake,
    Food,
    Free,
}

#[derive(Debug)]
pub struct SnakeGrid {
    grid: Vec<CellState>,
    grid_side: usize,
}

impl Index<Cell> for SnakeGrid {
    type Output = CellState;

    fn index(&self, cell: Cell) -> &Self::Output {
        &self.grid[self.index_of(cell)]
    }
}

impl IndexMut<Cell> for SnakeGrid {
    fn index_mut(&mut self, cell: Cell) -> &mut Self::Output {
        let i = self.index_of(cell);
        &mut self.grid[i]
    }
}

impl SnakeGrid {
    pub fn new(grid_side: usize, snake: &CircularSnake) -> Self {
        assert!(grid_side >= 4, "Grid is too small");
        let mut grid = Self {
            grid_side,
            grid: vec![CellState::Free; grid_side * grid_side],
        };

        for cell in &snake.snake_positions {
            grid[*cell] = CellState::Snake;
        }
        grid[snake.head()] = CellState::SnakeHead;
        grid
    }

    pub fn find_random_free_cell(&self) -> Option<Cell> {
        use rand::{rngs::StdRng, Rng, SeedableRng};

        // find a random free cell that will become food
        let mut rng = StdRng::seed_from_u64(1234);
        for _ in 0..60 {
            // don't loop infinitely
            let row = rng.gen_range(0..self.grid_side);
            let col = rng.gen_range(0..self.grid_side);
            if self[cell(row, col)] == CellState::Free {
                return Some(cell(row, col));
            }
            // otherwise try again
        }
        None
    }

    pub fn grid_side(&self) -> usize {
        self.grid_side
    }

    fn index_of(&self, cell: Cell) -> usize {
        cell.row * self.grid_side + cell.col
    }
}

pub mod output {
    use super::*;
    use crossterm::{
        cursor::MoveTo,
        execute,
        style::Stylize,
        terminal::{Clear, ClearType},
    };
    use std::io::{Result, Write};

    pub fn paint_on_raw_console(grid: &SnakeGrid) {
        let str = format_for_raw_console(grid).unwrap();
        let mut stdout = std::io::stdout();

        // Clear the terminal and write the new grid
        execute!(stdout, Clear(ClearType::All), MoveTo(0, 0)).unwrap();
        write!(stdout, "{}", str).unwrap();
        stdout.flush().unwrap();
    }

    fn print_fence(buf: &mut String, side: usize) -> Result<()> {
        buf.push('+');
        for _ in 0..side {
            buf.push_str("~~");
        }
        buf.push_str("+\n\r");
        Ok(())
    }

    fn format_for_raw_console(grid: &SnakeGrid) -> Result<String> {
        let mut buf = String::new();

        print_fence(&mut buf, grid.grid_side())?;

        // note: each cell is two chars wide because tty
        // characters are taller than they're wide. This
        // way vertical/horizontal speed is not warped.

        for row in 0..grid.grid_side() {
            buf.push('|');
            for col in 0..grid.grid_side() {
                let cell_str = match grid[cell(row, col)] {
                    CellState::SnakeHead => "@ ",
                    CellState::Snake => "o ",
                    CellState::Food => "x ",
                    CellState::Free => "  ",
                };

                // Colorize text based on cell state
                let colored_str = match grid[cell(row, col)] {
                    CellState::SnakeHead | CellState::Snake => cell_str.green(),
                    CellState::Food => cell_str.yellow(),
                    CellState::Free => cell_str.reset(),
                };

                buf.push_str(&format!("{}", colored_str));
            }
            buf.push_str("|\n\r");
        }

        print_fence(&mut buf, grid.grid_side())?;

        Ok(buf)
    }
}
