//! A track: the `.score` file (TOML) and its compiled form.
//!
//! A score is a few **cues** (sections the game moves between) made of **stems** (layers
//! that all play the same bars; the game only moves their faders), each played by a
//! **patch** (a small subtractive synth). The game's cues are `calm` (looping while he's
//! not engaged: suspicion rides the faders), `fight` (looping while he's engaged), and the
//! one-shot stingers `win` and `lose`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::pattern::{self, STEPS_PER_BAR, Step};
use super::theory::{self, Chord};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Score {
    pub title: String,
    /// Arena layouts this track is for (`yard`, `hall`, `l`, `cross`, `octagon`, `notch`,
    /// `classic`); empty means any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layouts: Vec<String>,
    /// Semitones added to every note: the same track in another key.
    #[serde(default, skip_serializing_if = "is_zero_i")]
    pub transpose: i32,
    /// Master volume.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub gain: f32,
    /// Reverb: how much of the mix goes to the hall (0..1) and its decay in seconds.
    #[serde(default = "default_reverb")]
    pub reverb: f32,
    #[serde(default = "default_room")]
    pub room: f32,
    #[serde(rename = "cue", default)]
    pub cues: Vec<Cue>,
    #[serde(rename = "stem", default)]
    pub stems: Vec<Stem>,
    #[serde(rename = "patch", default)]
    pub patches: BTreeMap<String, Patch>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Cue {
    pub name: String,
    pub bpm: f32,
    /// One chord per bar (`%` repeats). The cue is as long as this line.
    pub chords: String,
    /// How fast the faders move while this cue plays: the time constant in seconds.
    /// Small = the layers hit, large = they swell.
    #[serde(default = "default_fade")]
    pub fade: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stem {
    pub name: String,
    pub cue: String,
    pub patch: String,
    /// Fader at full.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub level: f32,
    /// How his suspicion moves this fader (calm cue only; other cues ignore it).
    #[serde(default, skip_serializing_if = "Fader::is_always")]
    pub fader: Fader,
    /// Play only the first time through the cue (an entrance hit).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub once: bool,
    /// The part: one token per 16th note (see `pattern`).
    pub part: String,
}

/// Fader rule: always up, or a smooth ramp across a suspicion range.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase", tag = "follow")]
pub enum Fader {
    #[default]
    Always,
    /// Silent below `from`, full above `to`.
    Rise { from: f32, to: f32 },
    /// Full below `from`, silent above `to`.
    Fall { from: f32, to: f32 },
}

impl Fader {
    pub fn is_always(&self) -> bool {
        *self == Fader::Always
    }
    /// 0..1 for a suspicion level.
    pub fn at(&self, suspicion: f32) -> f32 {
        match *self {
            Fader::Always => 1.0,
            Fader::Rise { from, to } => smooth(from, to, suspicion),
            Fader::Fall { from, to } => 1.0 - smooth(from, to, suspicion),
        }
    }
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    if b <= a {
        return if x >= a { 1.0 } else { 0.0 };
    }
    let k = ((x - a) / (b - a)).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Wave {
    #[default]
    Saw,
    SoftSaw,
    Square,
    Triangle,
    Sine,
    Organ,
    Noise,
}

impl Wave {
    pub const ALL: [Wave; 7] = [Wave::Saw, Wave::SoftSaw, Wave::Square, Wave::Triangle, Wave::Sine, Wave::Organ, Wave::Noise];
    pub fn name(self) -> &'static str {
        match self {
            Wave::Saw => "saw",
            Wave::SoftSaw => "soft saw",
            Wave::Square => "square",
            Wave::Triangle => "triangle",
            Wave::Sine => "sine",
            Wave::Organ => "organ",
            Wave::Noise => "noise",
        }
    }
}

/// One oscillator of a patch.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Osc {
    pub wave: Wave,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub semis: f32,
    /// Detune; a few saws a few cents apart make a section.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cents: f32,
    /// Frequency multiplier, for inharmonic partials (a bell's 4.01).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub ratio: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub level: f32,
}

impl Default for Osc {
    fn default() -> Self {
        Osc { wave: Wave::Saw, semis: 0.0, cents: 0.0, ratio: 1.0, level: 1.0 }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum FilterKind {
    #[default]
    None,
    Lowpass,
    Highpass,
    Bandpass,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub kind: FilterKind,
    pub cutoff: f32,
    #[serde(default = "default_q")]
    pub q: f32,
    /// Hz the cutoff opens by at the envelope's peak, then closes over `env_decay`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub env: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub env_attack: f32,
    #[serde(default = "default_env_decay", skip_serializing_if = "is_default_env_decay")]
    pub env_decay: f32,
}

impl Default for Filter {
    fn default() -> Self {
        Filter { kind: FilterKind::None, cutoff: 2000.0, q: default_q(), env: 0.0, env_attack: 0.0, env_decay: default_env_decay() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    /// What it's meant to sound like.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
    pub osc: Vec<Osc>,
    /// Amplitude envelope, seconds (sustain is a level 0..1). The note holds for as many
    /// steps as the part says, then releases.
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    #[serde(default = "one")]
    pub gain: f32,
    /// The pitch of `x` hits, as a chord-relative or absolute token (`r-24`, `d2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit: Option<String>,
    #[serde(default, skip_serializing_if = "Filter::is_off")]
    pub filter: Filter,
    /// Saturation before the filter (tanh drive; 0 = clean).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub drive: f32,
    /// Amplitude tremolo: rate in Hz and depth 0..1.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tremolo_rate: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tremolo_depth: f32,
    /// Semitones the pitch slides over the held note (risers).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub glide: f32,
    /// Drum thump: the pitch starts this much higher (×) and drops to the note in ~60 ms.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub drop: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pan: f32,
    /// Share sent to the reverb (0 = dry).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub send: f32,
}

impl Filter {
    pub fn is_off(&self) -> bool {
        self.kind == FilterKind::None
    }
}

impl Default for Patch {
    fn default() -> Self {
        Patch {
            about: String::new(),
            osc: vec![Osc::default()],
            attack: 0.01,
            decay: 0.2,
            sustain: 0.7,
            release: 0.3,
            gain: 1.0,
            hit: None,
            filter: Filter { kind: FilterKind::Lowpass, ..Filter::default() },
            drive: 0.0,
            tremolo_rate: 0.0,
            tremolo_depth: 0.0,
            glide: 0.0,
            drop: 0.0,
            pan: 0.0,
            send: 1.0,
        }
    }
}

fn one() -> f32 {
    1.0
}
fn is_one(x: &f32) -> bool {
    *x == 1.0
}
fn is_zero(x: &f32) -> bool {
    *x == 0.0
}
fn is_zero_i(x: &i32) -> bool {
    *x == 0
}
fn default_reverb() -> f32 {
    0.4
}
fn default_room() -> f32 {
    2.5
}
fn default_fade() -> f32 {
    0.7
}
fn default_q() -> f32 {
    0.7
}
fn default_env_decay() -> f32 {
    0.3
}
fn is_default_env_decay(x: &f32) -> bool {
    *x == default_env_decay()
}

/// What the game asks of a cue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Calm,
    Fight,
    Win,
    Lose,
    /// Not wired to the game (yet): plays only when picked in the editor.
    Other,
}

impl Role {
    pub fn of(name: &str) -> Role {
        match name {
            "calm" => Role::Calm,
            "fight" => Role::Fight,
            "win" => Role::Win,
            "lose" => Role::Lose,
            _ => Role::Other,
        }
    }
    /// Stingers play once; the rest loop.
    pub fn once(self) -> bool {
        matches!(self, Role::Win | Role::Lose)
    }
}

/// A cue ready to play.
#[derive(Clone, Debug)]
pub struct CompiledCue {
    pub name: String,
    pub role: Role,
    /// Seconds per 16th note.
    pub step: f64,
    pub chords: Vec<Chord>,
    pub fade: f32,
}

impl CompiledCue {
    pub fn bars(&self) -> usize {
        self.chords.len()
    }
    pub fn steps(&self) -> usize {
        self.bars() * STEPS_PER_BAR
    }
}

/// A stem ready to play.
#[derive(Clone, Debug)]
pub struct CompiledStem {
    pub name: String,
    pub cue: usize,
    pub patch: Patch,
    /// Parsed `hit` token.
    pub hit: Vec<pattern::Pitch>,
    pub level: f32,
    pub fader: Fader,
    pub once: bool,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug)]
pub struct Compiled {
    pub transpose: i32,
    pub gain: f32,
    pub reverb: f32,
    pub room: f32,
    pub cues: Vec<CompiledCue>,
    pub stems: Vec<CompiledStem>,
}

impl Compiled {
    pub fn cue(&self, role: Role) -> Option<usize> {
        self.cues.iter().position(|c| c.role == role)
    }
}

impl Score {
    pub fn parse(text: &str) -> Result<Score, String> {
        let score: Score = toml::from_str(text).map_err(|e| e.to_string())?;
        score.compile()?;
        Ok(score)
    }

    /// The file as it's saved: sections in a fixed order, stems grouped by cue, small tables
    /// inline, numbers at the precision they were typed.
    pub fn to_text(&self) -> String {
        use std::fmt::Write;
        let v = toml::Value::try_from(self).expect("a score always serializes");
        let top = v.as_table().expect("a table");
        let mut out = String::new();
        for k in ["title", "layouts", "transpose", "gain", "reverb", "room"] {
            if let Some(x) = top.get(k) {
                let _ = writeln!(out, "{k} = {}", inline(x));
            }
        }
        let rule = |out: &mut String, title: &str| {
            let _ = write!(out, "\n# ── {title} {}\n", "─".repeat(76usize.saturating_sub(title.chars().count())));
        };
        let table = |out: &mut String, header: &str, t: &toml::Value| {
            let _ = writeln!(out, "\n{header}");
            for (k, x) in ordered(t.as_table().expect("a table")) {
                let _ = writeln!(out, "{k} = {}", inline(x));
            }
        };
        let cues = top.get("cue").and_then(|c| c.as_array()).cloned().unwrap_or_default();
        let stems = top.get("stem").and_then(|c| c.as_array()).cloned().unwrap_or_default();
        rule(&mut out, "Cues");
        for c in &cues {
            table(&mut out, "[[cue]]", c);
        }
        // Stems under their cue, in cue order; any stem whose cue is missing goes last.
        let mut names: Vec<&str> = self.cues.iter().map(|c| c.name.as_str()).collect();
        for s in &self.stems {
            if !names.contains(&s.cue.as_str()) {
                names.push(&s.cue);
            }
        }
        for name in names {
            let mine: Vec<_> = self.stems.iter().zip(&stems).filter(|(s, _)| s.cue == name).collect();
            if mine.is_empty() {
                continue;
            }
            rule(&mut out, &format!("Stems: {name}"));
            for (_, v) in mine {
                table(&mut out, "[[stem]]", v);
            }
        }
        rule(&mut out, "Patches");
        if let Some(patches) = top.get("patch").and_then(|p| p.as_table()) {
            for (name, p) in patches {
                table(&mut out, &format!("[patch.{}]", key(name)), p);
            }
        }
        out
    }

    pub fn compile(&self) -> Result<Compiled, String> {
        let mut cues = Vec::new();
        for c in &self.cues {
            if !(20.0..=300.0).contains(&c.bpm) {
                return Err(format!("cue `{}`: bpm {} is out of range (20..300)", c.name, c.bpm));
            }
            if cues.iter().any(|x: &CompiledCue| x.name == c.name) {
                return Err(format!("two cues are called `{}`", c.name));
            }
            cues.push(CompiledCue {
                name: c.name.clone(),
                role: Role::of(&c.name),
                step: 60.0 / c.bpm as f64 / 4.0,
                chords: theory::chord_line(&c.chords).map_err(|e| format!("cue `{}`: {e}", c.name))?,
                fade: c.fade.max(0.005),
            });
        }
        if !cues.iter().any(|c| c.role == Role::Calm) {
            return Err("a score needs a `calm` cue".into());
        }
        let mut stems = Vec::new();
        for s in &self.stems {
            let ctx = |e: String| format!("stem `{}`: {e}", s.name);
            let cue = cues.iter().position(|c| c.name == s.cue).ok_or_else(|| ctx(format!("no cue called `{}`", s.cue)))?;
            let patch = self.patches.get(&s.patch).ok_or_else(|| ctx(format!("no patch called `{}`", s.patch)))?;
            let tokens = pattern::tokens(&s.part).map_err(ctx)?;
            if tokens.is_empty() {
                return Err(ctx("the part is empty".into()));
            }
            let hit = match &patch.hit {
                Some(h) => match pattern::step(h).map_err(|e| ctx(format!("patch `{}` hit: {e}", s.patch)))? {
                    Step::Notes(p, _) => p,
                    _ => return Err(ctx(format!("patch `{}`: hit must be a note", s.patch))),
                },
                None => vec![pattern::Pitch::Root(0)],
            };
            stems.push(CompiledStem {
                name: s.name.clone(),
                cue,
                patch: patch.clone(),
                hit,
                level: s.level,
                fader: s.fader,
                once: s.once,
                steps: tokens.iter().map(|t| pattern::step(t)).collect::<Result<_, _>>().map_err(ctx)?,
            });
        }
        Ok(Compiled { transpose: self.transpose, gain: self.gain, reverb: self.reverb, room: self.room, cues, stems })
    }
}

/// Field order in the file (anything else follows, alphabetically).
const ORDER: [&str; 43] = [
    "name", "cue", "patch", "about", "follow", "wave", "kind", "bpm", "chords", "once", "fade", "from", "to",
    "semis", "cents", "ratio", "level", "osc", "cutoff", "q", "env", "env_attack", "env_decay", "attack", "decay", "sustain",
    "release", "gain", "hit", "drive", "filter", "tremolo_rate", "tremolo_depth", "glide", "drop", "pan", "send",
    "fader", "part", "title", "layouts", "transpose", "reverb",
];

fn ordered(t: &toml::Table) -> Vec<(&String, &toml::Value)> {
    let mut v: Vec<_> = t.iter().collect();
    v.sort_by_key(|(k, _)| (ORDER.iter().position(|o| o == k).unwrap_or(ORDER.len()), k.to_string()));
    v
}

fn key(k: &str) -> String {
    if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        k.to_string()
    } else {
        toml::Value::String(k.to_string()).to_string()
    }
}

fn inline(v: &toml::Value) -> String {
    match v {
        // Our numbers are f32: print them at f32 precision, so 0.07 stays 0.07.
        toml::Value::Float(f) => {
            let s = format!("{}", *f as f32);
            if s.contains(['.', 'e', 'i', 'N']) { s } else { format!("{s}.0") }
        }
        toml::Value::Array(a) => format!("[{}]", a.iter().map(inline).collect::<Vec<_>>().join(", ")),
        toml::Value::Table(t) => {
            let parts: Vec<String> = ordered(t).into_iter().map(|(k, x)| format!("{} = {}", key(k), inline(x))).collect();
            format!("{{ {} }}", parts.join(", "))
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faders() {
        let r = Fader::Rise { from: 0.2, to: 0.6 };
        assert_eq!(r.at(0.0), 0.0);
        assert!((r.at(0.4) - 0.5).abs() < 1e-5);
        assert_eq!(r.at(1.0), 1.0);
        let f = Fader::Fall { from: 0.2, to: 0.6 };
        assert_eq!(f.at(0.0), 1.0);
        assert_eq!(f.at(1.0), 0.0);
        assert_eq!(Fader::Always.at(0.3), 1.0);
    }
}
