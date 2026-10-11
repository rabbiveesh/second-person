//! Pitch and harmony: note names, chord symbols and the voicings patterns read from.

/// MIDI note number as a float, so glides and detune stay continuous.
pub type Midi = f32;

/// Pitch class (0 = C) of a note letter with optional accidentals: `C`, `F#`, `Bb`, `Ebb`.
/// Returns the class and how many bytes it consumed.
fn pitch_class(s: &str) -> Option<(i32, usize)> {
    let b = s.as_bytes();
    let base: i32 = match b.first()?.to_ascii_uppercase() {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let mut pc = base;
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'#' => pc += 1,
            b'b' => pc -= 1,
            _ => break,
        }
        i += 1;
    }
    Some((pc.rem_euclid(12), i))
}

/// An absolute note name with its octave, like `d4`, `F#5`, `bb2` (C4 = MIDI 60).
pub fn note(s: &str) -> Option<i32> {
    let (pc, used) = pitch_class(s)?;
    // The octave keeps its sign: `c-1` is MIDI 0.
    let rest = &s[used..];
    let octave: i32 = rest.parse().ok()?;
    // Spell-out like `Cb4` / `B#3` wraps across the octave line.
    let base = match s[..used].as_bytes() {
        [b'c' | b'C', b'b', ..] => -12,
        [b'b' | b'B', b'#', ..] => 12,
        _ => 0,
    };
    Some((octave + 1) * 12 + pc + base)
}

/// Name a MIDI note, sharps only (`a#3`).
pub fn note_name(m: i32) -> String {
    const N: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    format!("{}{}", N[m.rem_euclid(12) as usize], m.div_euclid(12) - 1)
}

/// A chord as patterns see it: the root (placed between A2 and G#3, the cellos' sweet
/// spot) and its tones as intervals above the root.
#[derive(Clone, Debug, PartialEq)]
pub struct Chord {
    pub root: i32,
    /// Root, third, fifth, then any seventh and extensions, ascending.
    pub tones: Vec<i32>,
}

impl Chord {
    /// Parse a chord symbol: `D`, `Dm`, `Bb`, `A7`, `Am7`, `Cmaj7`, `Bdim`, `Bm7b5`, `Caug`,
    /// `Dsus4`, `Dsus2`, `D5`, `A7b9`, `Dm9`, `Dmadd9`.
    pub fn parse(s: &str) -> Option<Chord> {
        let (pc, used) = pitch_class(s)?;
        let q = &s[used..];
        let root = 45 + (pc - 9).rem_euclid(12);
        // Strip the extensions we know off the end, building the stack as we go.
        let (triad, rest) = if let Some(r) = q.strip_prefix("maj") {
            (vec![0, 4, 7], ("maj", r))
        } else if let Some(r) = q.strip_prefix("min") {
            (vec![0, 3, 7], ("", r))
        } else if let Some(r) = q.strip_prefix("dim") {
            (vec![0, 3, 6], ("dim", r))
        } else if let Some(r) = q.strip_prefix("aug").or_else(|| q.strip_prefix('+')) {
            (vec![0, 4, 8], ("", r))
        } else if let Some(r) = q.strip_prefix("sus2") {
            (vec![0, 2, 7], ("", r))
        } else if let Some(r) = q.strip_prefix("sus4").or_else(|| q.strip_prefix("sus")) {
            (vec![0, 5, 7], ("", r))
        } else if let Some(r) = q.strip_prefix('m') {
            (vec![0, 3, 7], ("", r))
        } else if let Some(r) = q.strip_prefix('5') {
            (vec![0, 7], ("", r))
        } else {
            (vec![0, 4, 7], ("", q))
        };
        let mut tones = triad;
        let (kind, mut ext) = rest;
        // Sevenths.
        if kind == "maj" {
            if let Some(r) = ext.strip_prefix('7') {
                tones.push(11);
                ext = r;
            } else if let Some(r) = ext.strip_prefix('9') {
                tones.extend([11, 14]);
                ext = r;
            } else if ext.is_empty() {
                // Plain `Cmaj` is a major triad.
            }
        } else if let Some(r) = ext.strip_prefix('7') {
            tones.push(if kind == "dim" { 9 } else { 10 });
            ext = r;
        } else if let Some(r) = ext.strip_prefix('9') {
            tones.extend([10, 14]);
            ext = r;
        } else if let Some(r) = ext.strip_prefix('6') {
            tones.push(9);
            ext = r;
        }
        // Alterations and additions, in any order.
        while !ext.is_empty() {
            let (add, r) = if let Some(r) = ext.strip_prefix("b5") {
                tones.retain(|&t| t != 7);
                (6, r)
            } else if let Some(r) = ext.strip_prefix("#5") {
                tones.retain(|&t| t != 7);
                (8, r)
            } else if let Some(r) = ext.strip_prefix("b9") {
                (13, r)
            } else if let Some(r) = ext.strip_prefix("#9") {
                (15, r)
            } else if let Some(r) = ext.strip_prefix("add9") {
                (14, r)
            } else if let Some(r) = ext.strip_prefix("#11") {
                (18, r)
            } else if let Some(r) = ext.strip_prefix("11") {
                (17, r)
            } else if let Some(r) = ext.strip_prefix("b13") {
                (20, r)
            } else if let Some(r) = ext.strip_prefix("13") {
                (21, r)
            } else {
                return None;
            };
            tones.push(add);
            ext = r;
        }
        tones.sort_unstable();
        tones.dedup();
        Some(Chord { root, tones })
    }

    /// The `n`th chord tone (1 = root, 2 = third, 3 = fifth, 4 = seventh, 5 = ninth...),
    /// in semitones above the root. Past the top of the stack it wraps up an octave.
    pub fn tone(&self, n: usize) -> i32 {
        let i = n.max(1) - 1;
        let len = self.tones.len();
        self.tones[i % len] + 12 * (i / len) as i32
    }

    /// A full, open voicing for pads: root, fifth, octave, then the third and any
    /// seventh/extensions an octave up. `D` gives D3 A3 D4 F#4.
    pub fn voicing(&self) -> Vec<i32> {
        let mut v = vec![self.root];
        let fifth = self.tones.iter().find(|&&t| (6..=8).contains(&t)).copied();
        if let Some(f) = fifth {
            v.push(self.root + f);
        }
        v.push(self.root + 12);
        for &t in &self.tones {
            if t != 0 && Some(t) != fifth {
                v.push(self.root + 12 + t);
            }
        }
        v
    }
}

/// Parse a cue's chord line: one symbol per bar, `%` repeats the previous bar, `|` is
/// ignored (write it for readability).
pub fn chord_line(s: &str) -> Result<Vec<Chord>, String> {
    let mut out: Vec<Chord> = Vec::new();
    for tok in s.split_whitespace().filter(|t| *t != "|") {
        if tok == "%" {
            let prev = out.last().cloned().ok_or("`%` needs a chord before it")?;
            out.push(prev);
        } else {
            out.push(Chord::parse(tok).ok_or_else(|| format!("unknown chord `{tok}`"))?);
        }
    }
    if out.is_empty() {
        return Err("no chords".into());
    }
    Ok(out)
}

pub fn hz(m: Midi) -> f32 {
    440.0 * 2f32.powf((m - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes() {
        assert_eq!(note("c4"), Some(60));
        assert_eq!(note("D3"), Some(50));
        assert_eq!(note("bb2"), Some(46));
        assert_eq!(note("f#5"), Some(78));
        assert_eq!(note("c-1"), Some(0));
        assert_eq!(note("x4"), None);
        assert_eq!(note_name(50), "d3");
    }

    #[test]
    fn chords_sit_where_the_sketch_had_them() {
        let d = Chord::parse("Dm").unwrap();
        assert_eq!(d.root, 50);
        assert_eq!(d.tones, vec![0, 3, 7]);
        assert_eq!(d.voicing(), vec![50, 57, 62, 65]);
        assert_eq!(Chord::parse("Bb").unwrap().root, 46);
        assert_eq!(Chord::parse("A").unwrap().root, 45);
        assert_eq!(Chord::parse("Eb").unwrap().root, 51);
        assert_eq!(Chord::parse("A7b9").unwrap().tones, vec![0, 4, 7, 10, 13]);
        assert_eq!(Chord::parse("Bm7b5").unwrap().tones, vec![0, 3, 6, 10]);
        assert_eq!(Chord::parse("Cmaj7").unwrap().tones, vec![0, 4, 7, 11]);
        assert_eq!(Chord::parse("Dsus4").unwrap().tones, vec![0, 5, 7]);
        assert_eq!(Chord::parse("Dsus2").unwrap().tones, vec![0, 2, 7]);
        assert!(Chord::parse("Dzz").is_none());
    }

    #[test]
    fn tones_wrap_up_the_octave() {
        let d = Chord::parse("Dm").unwrap();
        assert_eq!(d.tone(1), 0);
        assert_eq!(d.tone(2), 3);
        assert_eq!(d.tone(3), 7);
        assert_eq!(d.tone(4), 12);
        assert_eq!(d.tone(5), 15);
    }

    #[test]
    fn chord_lines() {
        let c = chord_line("Dm % | Bb A").unwrap();
        assert_eq!(c.len(), 4);
        assert_eq!(c[1], c[0]);
        assert!(chord_line("% Dm").is_err());
    }
}
