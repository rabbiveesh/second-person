//! A scripted round for previews and offline renders: he hears you, loses you, spots you,
//! fights, and you win.

/// Suspicion keyframes (seconds, level), linearly interpolated.
const KEYS: [(f32, f32); 7] = [(0.0, 0.0), (4.0, 0.0), (10.0, 0.55), (14.0, 0.55), (18.0, 0.25), (22.0, 0.25), (27.0, 0.95)];
pub const ENGAGE_AT: f32 = 27.0;
pub const WIN_AT: f32 = 46.0;
pub const LENGTH: f32 = 54.0;

/// The game state at `t` seconds into the round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moment {
    pub suspicion: f32,
    pub engaged: bool,
    pub won: bool,
}

pub fn at(t: f32) -> Moment {
    let mut suspicion = KEYS[KEYS.len() - 1].1;
    for w in KEYS.windows(2) {
        let ((a, va), (b, vb)) = (w[0], w[1]);
        if t >= a && t < b {
            suspicion = va + (vb - va) * (t - a) / (b - a);
        }
    }
    let engaged = (ENGAGE_AT..WIN_AT).contains(&t);
    if engaged {
        suspicion = 1.0;
    }
    Moment { suspicion, engaged, won: t >= WIN_AT }
}

/// What a moment looks like to a viewer.
pub fn caption(m: Moment) -> &'static str {
    if m.won {
        "He's down"
    } else if m.engaged {
        "Engaged"
    } else if m.suspicion > 0.6 {
        "He's almost sure"
    } else if m.suspicion > 0.3 {
        "He heard something"
    } else {
        "He's looking around"
    }
}
