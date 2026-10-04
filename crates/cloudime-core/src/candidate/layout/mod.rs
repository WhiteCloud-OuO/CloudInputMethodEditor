//! 候选窗口的排布：本地候选按页排。
//!
//! 这是展示规则不是排序规则，但 CLI 与 Windows 壳都要用同一套，所以放在 Core。

mod candidate_layout;
mod grid;

pub use candidate_layout::CandidateLayout;
pub use grid::{GRID_ROWS, Grid, MAX_CELL_EMS};
