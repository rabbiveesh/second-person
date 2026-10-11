//! One note of a patch as a fundsp graph: oscillators → drive → filter → envelope → pan.

use fundsp::prelude32::{
    AudioUnit, Net, Tanh, bandpass, dc, highpass, lfo, lowpass, noise, organ, pan, saw, shape, sine, soft_saw, square, triangle, zero,
};

use super::score::{FilterKind, Patch, Wave};
use super::theory::hz;

/// How long a note held `gate` seconds sounds in all. A patch with no sustain is a
/// one-shot (plucks, bells, drums): it rings out its decay whatever the gate.
pub fn length(patch: &Patch, gate: f64) -> f64 {
    if patch.sustain <= 0.0 {
        (patch.attack.max(0.001) + patch.decay.max(0.001) * 1.6) as f64
    } else {
        gate + patch.release.max(0.005) as f64 + 0.02
    }
}

/// The amplitude envelope at `t` seconds for a note held `gate` seconds.
pub fn amp(patch: &Patch, gate: f32, t: f32) -> f32 {
    let a = patch.attack.max(0.001);
    let d = patch.decay.max(0.001);
    let s = patch.sustain.clamp(0.0, 1.0);
    let held = |t: f32| {
        if t < a {
            t / a
        } else {
            // Exponential-ish decay to the sustain level.
            s + (1.0 - s) * (-(t - a) / d * 3.0).exp()
        }
    };
    if t < gate || s <= 0.0 {
        held(t)
    } else {
        let r = patch.release.max(0.005);
        held(gate) * (1.0 - (t - gate) / r).max(0.0).powi(2)
    }
}

/// Build the note. `vel` scales the level and, for accents, opens the filter a little.
pub fn note(patch: &Patch, midi: f32, gate: f64, vel: f32) -> Box<dyn AudioUnit> {
    let f0 = hz(midi);
    let gate = gate as f32;
    let glide = patch.glide;
    let drop = patch.drop;
    let pitch = move |t: f32| {
        let mut f = f0 * 2f32.powf(glide * (t / gate.max(0.001)).min(1.0) / 12.0);
        if drop > 0.0 {
            f *= 1.0 + drop * (-t / 0.02).exp();
        }
        f
    };

    let mut src: Option<Net> = None;
    for o in &patch.osc {
        let ratio = o.ratio * 2f32.powf((o.semis + o.cents / 100.0) / 12.0);
        let freq = lfo(move |t: f32| pitch(t) * ratio);
        let n = match o.wave {
            Wave::Saw => Net::wrap(Box::new(freq >> saw())),
            Wave::SoftSaw => Net::wrap(Box::new(freq >> soft_saw())),
            Wave::Square => Net::wrap(Box::new(freq >> square())),
            Wave::Triangle => Net::wrap(Box::new(freq >> triangle())),
            Wave::Sine => Net::wrap(Box::new(freq >> sine())),
            Wave::Organ => Net::wrap(Box::new(freq >> organ())),
            Wave::Noise => Net::wrap(Box::new(noise())),
        } * o.level;
        src = Some(match src {
            Some(s) => s + n,
            None => n,
        });
    }
    let mut net = src.unwrap_or_else(|| Net::wrap(Box::new(zero())));
    if patch.drive > 0.0 {
        let k = patch.drive;
        net = net >> shape(Tanh(k)) * (1.0 / k.tanh().max(0.2));
    }
    let fl = &patch.filter;
    if fl.kind != FilterKind::None {
        let (base, amount, fa, fd) = (fl.cutoff, fl.env * vel.max(1.0), fl.env_attack.max(0.0), fl.env_decay.max(0.005));
        let cutoff = lfo(move |t: f32| {
            let e = if t < fa { t / fa.max(1e-4) } else { (-(t - fa) / fd).exp() };
            (base + amount * e).clamp(20.0, 18000.0)
        });
        let q = dc(fl.q.max(0.1));
        net = match fl.kind {
            FilterKind::Lowpass => (net | cutoff | q) >> lowpass(),
            FilterKind::Highpass => (net | cutoff | q) >> highpass(),
            FilterKind::Bandpass => (net | cutoff | q) >> bandpass(),
            FilterKind::None => unreachable!(),
        };
    }
    let p = patch.clone();
    let level = patch.gain * vel;
    let (tr, td) = (patch.tremolo_rate, patch.tremolo_depth.clamp(0.0, 1.0));
    let env = lfo(move |t: f32| {
        let trem = if td > 0.0 { 1.0 - td * 0.5 * (1.0 - (std::f32::consts::TAU * tr * t).cos()) } else { 1.0 };
        amp(&p, gate, t) * trem * level
    });
    let net = (net * env) >> pan(patch.pan.clamp(-1.0, 1.0));
    Box::new(net)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::score::{Filter, Osc};

    fn rms(unit: &mut dyn AudioUnit, secs: f64) -> f32 {
        unit.set_sample_rate(48000.0);
        unit.allocate();
        let n = (48000.0 * secs) as usize;
        let mut sum = 0.0;
        for _ in 0..n {
            let (l, r) = unit.get_stereo();
            sum += l * l + r * r;
        }
        (sum / (2 * n) as f32).sqrt()
    }

    #[test]
    fn a_note_sounds_then_dies() {
        let patch = Patch {
            osc: vec![Osc { wave: Wave::Saw, ..Osc::default() }, Osc { wave: Wave::Saw, cents: 8.0, ..Osc::default() }],
            filter: Filter { kind: FilterKind::Lowpass, cutoff: 1200.0, env: 800.0, ..Filter::default() },
            drive: 1.5,
            tremolo_rate: 12.0,
            tremolo_depth: 0.5,
            ..Patch::default()
        };
        let mut unit = note(&patch, 57.0, 0.5, 1.0);
        let on = rms(&mut *unit, 0.5);
        assert!(on > 0.05, "too quiet: {on}");
        let tail = rms(&mut *unit, 0.32);
        let after = rms(&mut *unit, 0.2);
        assert!(tail < on, "the release should fall: {tail} vs {on}");
        assert!(after < 1e-4, "silent after the release: {after}");
    }

    #[test]
    fn envelope_shape() {
        let p = Patch { attack: 0.1, decay: 0.1, sustain: 0.5, release: 0.2, ..Patch::default() };
        assert_eq!(amp(&p, 1.0, 0.0), 0.0);
        assert!((amp(&p, 1.0, 0.1) - 1.0).abs() < 1e-5);
        assert!((amp(&p, 1.0, 0.9) - 0.5).abs() < 0.01);
        assert_eq!(amp(&p, 1.0, 1.3), 0.0);
        // No sustain: a one-shot that ignores the gate.
        let pluck = Patch { attack: 0.01, decay: 0.3, sustain: 0.0, ..Patch::default() };
        assert!(amp(&pluck, 0.05, 0.1) > 0.3);
        assert!(length(&pluck, 0.05) > 0.4);
    }
}
