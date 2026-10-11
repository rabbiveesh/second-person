//! HIM's adaptive score: tracks (`music/*.score`) that follow his state, synthesized live.
//! Pure and Bevy-free; see `engine` for how the game drives it.

pub mod engine;
pub mod pattern;
pub mod score;
pub mod script;
pub mod theory;
pub mod voice;

pub use engine::{Engine, Status};
pub use score::{Compiled, Score};

/// Where tracks live, relative to the repo root.
pub const DIR: &str = "music";
