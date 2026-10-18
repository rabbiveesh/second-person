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

/// The soundfont sampled patches play from, relative to the repo root.
pub const SOUNDFONT: &str = "assets/music/gm.sf2";

pub use rustysynth::SoundFont;

/// Read a soundfont (SF2) for `Engine::set_soundfont`.
pub fn load_soundfont(path: impl AsRef<std::path::Path>) -> Result<std::sync::Arc<SoundFont>, String> {
    let path = path.as_ref();
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    SoundFont::new(&mut file).map(std::sync::Arc::new).map_err(|e| format!("{}: {e}", path.display()))
}
