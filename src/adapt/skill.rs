//! What a round asks of you, and how hard.

/// Difficulty band of one skill: 1 (gentlest) ..= 10 (hardest).
pub type Band = u8;
pub const MIN_BAND: Band = 1;
pub const MAX_BAND: Band = 10;
/// The band at which his tuning is the hand-tuned game's.
pub const BASE_BAND: Band = 5;

/// One thing a round tests. Each has its own band, spread and window, and one round usually
/// gives evidence on both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Skill {
    /// Getting the first hit in before he engages. Bands his senses: view cone, how fast
    /// suspicion fills, how far he hears.
    Stealth,
    /// Winning once he's engaged. Bands his gunplay: fire rate and damage, holding angles,
    /// grapple, dodge roll, hit points.
    Gunfight,
}

impl Skill {
    pub const ALL: [Skill; 2] = [Skill::Stealth, Skill::Gunfight];
    pub const COUNT: usize = Self::ALL.len();

    /// Index into per-skill arrays (`ALL[s.index()] == s`).
    pub fn index(self) -> usize {
        self as usize
    }

    /// Short lowercase name (simulator output, logs). Never shown to the player.
    pub fn name(self) -> &'static str {
        match self {
            Skill::Stealth => "stealth",
            Skill::Gunfight => "gunfight",
        }
    }
}

/// Clamp any integer into `MIN_BAND..=MAX_BAND`.
pub fn clamp_band(b: i32) -> Band {
    b.clamp(MIN_BAND as i32, MAX_BAND as i32) as Band
}
