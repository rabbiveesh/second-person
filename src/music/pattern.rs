//! A stem's part: one token per 16th note, read against the bar's chord.
//!
//! | token | plays |
//! |---|---|
//! | `.` | rest |
//! | `_` | hold the previous note one more step |
//! | `x` | a hit at the patch's own pitch (drums) |
//! | `r` `r-12` `r+7` | the chord root, offset in semitones |
//! | `1` `2` `3` `4` `3+12` | chord tones: root, third, fifth, seventh (wraps up the octave) |
//! | `ch` `ch+12` | the whole chord, voiced open |
//! | `d4` `bb2` `f#5` | an absolute note |
//! | `r-12,r,3` | stack several with commas |
//! | `!` / `?` suffix | accent / ghost (`X` is an accented `x`) |
//! | `tok*n` | the token `n` times (`_*63` holds four bars) |
//!
//! `|` is ignored; write it between bars. A part shorter than its cue loops, so a one-bar
//! figure under a four-bar progression follows the chords.

use super::theory::{self, Chord};

pub const STEPS_PER_BAR: usize = 16;
pub const ACCENT: f32 = 1.35;
pub const GHOST: f32 = 0.55;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pitch {
    /// The patch's own pitch (`x`).
    Base,
    /// The chord root plus semitones.
    Root(i32),
    /// Chord tone `n` (1-based) plus semitones.
    Tone(u8, i32),
    /// The open chord voicing plus semitones.
    Voicing(i32),
    Abs(i32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Rest,
    Hold,
    Notes(Vec<Pitch>, f32),
}

/// Parse one (unrepeated) token.
pub fn step(tok: &str) -> Result<Step, String> {
    match tok {
        "." => return Ok(Step::Rest),
        "_" => return Ok(Step::Hold),
        "x" => return Ok(Step::Notes(vec![Pitch::Base], 1.0)),
        "X" => return Ok(Step::Notes(vec![Pitch::Base], ACCENT)),
        _ => {}
    }
    let (body, vel) = if let Some(b) = tok.strip_suffix('!') {
        (b, ACCENT)
    } else if let Some(b) = tok.strip_suffix('?') {
        (b, GHOST)
    } else {
        (tok, 1.0)
    };
    let mut pitches = Vec::new();
    for part in body.split(',') {
        pitches.push(pitch(part).ok_or_else(|| format!("can't read `{part}` in `{tok}`"))?);
    }
    Ok(Step::Notes(pitches, vel))
}

fn pitch(p: &str) -> Option<Pitch> {
    if p == "x" {
        return Some(Pitch::Base);
    }
    let b = p.as_bytes();
    let head_len = if p.starts_with("ch") {
        2
    } else if p.starts_with('r') || b.first()?.is_ascii_digit() {
        1
    } else {
        // An absolute note: letter, accidentals, then the octave (which may be `-1`).
        let mut i = 1;
        while i < b.len() && matches!(b[i], b'#' | b'b') {
            i += 1;
        }
        if i + 1 < b.len() && b[i] == b'-' && b[i + 1].is_ascii_digit() {
            i += 1;
        }
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        i
    };
    let (head, rest) = p.split_at(head_len);
    let off: i32 = if rest.is_empty() { 0 } else { rest.parse().ok()? };
    match head {
        "r" => Some(Pitch::Root(off)),
        "ch" => Some(Pitch::Voicing(off)),
        _ if b[0].is_ascii_digit() => {
            let n = b[0] - b'0';
            (n >= 1).then_some(Pitch::Tone(n, off))
        }
        _ => theory::note(head).map(|m| Pitch::Abs(m + off)),
    }
}

/// Split a part into its tokens, expanding `tok*n`.
pub fn tokens(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for t in s.split_whitespace().filter(|t| *t != "|") {
        if let Some((tok, n)) = t.rsplit_once('*') {
            let n: usize = n.parse().map_err(|_| format!("bad repeat in `{t}`"))?;
            if n == 0 || n > 4096 {
                return Err(format!("repeat out of range in `{t}`"));
            }
            step(tok)?;
            out.extend(std::iter::repeat_n(tok.to_string(), n));
        } else {
            step(t)?;
            out.push(t.to_string());
        }
    }
    Ok(out)
}

/// Write tokens back: runs of three or more collapse to `tok*n`, and a `|` marks each bar
/// line that doesn't fall inside a run.
pub fn format(tokens: &[String]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < tokens.len() {
        let mut j = i + 1;
        while j < tokens.len() && tokens[j] == tokens[i] {
            j += 1;
        }
        // Keep a run whole only when it's long enough to be worth it; otherwise write the
        // tokens out so the bar lines stay visible.
        let run = j - i;
        let collapse = run >= 3;
        let end = if collapse { j } else { i + 1 };
        if i > 0 {
            out.push(' ');
            if i % STEPS_PER_BAR == 0 {
                out.push_str("| ");
            }
        }
        if collapse {
            out.push_str(&format!("{}*{run}", tokens[i]));
        } else {
            out.push_str(&tokens[i]);
        }
        i = end;
    }
    out
}

/// The notes a step plays against a chord, as MIDI numbers.
pub fn resolve(pitches: &[Pitch], chord: &Chord, base: i32) -> Vec<i32> {
    let mut out = Vec::new();
    for p in pitches {
        match *p {
            Pitch::Base => out.push(base),
            Pitch::Root(o) => out.push(chord.root + o),
            Pitch::Tone(n, o) => out.push(chord.root + chord.tone(n as usize) + o),
            Pitch::Voicing(o) => out.extend(chord.voicing().into_iter().map(|m| m + o)),
            Pitch::Abs(m) => out.push(m),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_kind_of_token() {
        assert_eq!(step(".").unwrap(), Step::Rest);
        assert_eq!(step("_").unwrap(), Step::Hold);
        assert_eq!(step("X").unwrap(), Step::Notes(vec![Pitch::Base], ACCENT));
        assert_eq!(step("r-12").unwrap(), Step::Notes(vec![Pitch::Root(-12)], 1.0));
        assert_eq!(step("r+7!").unwrap(), Step::Notes(vec![Pitch::Root(7)], ACCENT));
        assert_eq!(step("3+12?").unwrap(), Step::Notes(vec![Pitch::Tone(3, 12)], GHOST));
        assert_eq!(step("ch").unwrap(), Step::Notes(vec![Pitch::Voicing(0)], 1.0));
        assert_eq!(step("d4").unwrap(), Step::Notes(vec![Pitch::Abs(62)], 1.0));
        assert_eq!(step("c-1").unwrap(), Step::Notes(vec![Pitch::Abs(0)], 1.0));
        assert_eq!(step("bb2+12").unwrap(), Step::Notes(vec![Pitch::Abs(58)], 1.0));
        assert_eq!(step("r-12,r,3").unwrap(), Step::Notes(vec![Pitch::Root(-12), Pitch::Root(0), Pitch::Tone(3, 0)], 1.0));
        assert!(step("q").is_err());
        assert!(step("0").is_err());
    }

    #[test]
    fn repeats_expand_and_format_round_trips() {
        let t = tokens("ch _*15 | . . r r+1").unwrap();
        assert_eq!(t.len(), 20);
        assert_eq!(t[1], "_");
        let f = format(&t);
        assert_eq!(f, "ch _*15 | . . r r+1");
        assert_eq!(tokens(&f).unwrap(), t);
        assert!(tokens("r*0").is_err());
        assert!(tokens("zz*2").is_err());
    }

    #[test]
    fn resolves_against_the_chord() {
        let dm = Chord::parse("Dm").unwrap();
        assert_eq!(resolve(&[Pitch::Root(-12)], &dm, 0), vec![38]);
        assert_eq!(resolve(&[Pitch::Tone(2, 12)], &dm, 0), vec![65]);
        assert_eq!(resolve(&[Pitch::Voicing(0)], &dm, 0), vec![50, 57, 62, 65]);
        assert_eq!(resolve(&[Pitch::Base], &dm, 36), vec![36]);
    }
}
