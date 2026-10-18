//! Regenerates the procedural sound effects in `assets/sfx/` with sfxr.
//!
//!     cargo run --example gen_sfx
//!
//! Seeds are fixed so the output is reproducible; tweak a seed or parameter and rerun.
//! Every sound also gets a low-passed `<name>_muffled.wav` twin, which `audio.rs` crossfades
//! in for sounds behind the listener or behind cover.

use rand::{Rng, SeedableRng, rngs::StdRng};
use second_person::arena::Floor;
use sfxr::{Generator, Sample, WaveType};

const RATE: u32 = 44_100;

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/sfx");
    std::fs::create_dir_all(&dir).unwrap();

    let sounds: Vec<(&str, Sample)> = vec![
        ("shot", {
            let mut s = Sample::explosion(Some(7));
            s.env_sustain = 0.08;
            s.env_decay = 0.25;
            s.base_freq = 0.35;
            s
        }),
        ("impact", Sample::hit(Some(3))),
        ("target_hit", {
            let mut s = Sample::hit(Some(11));
            s.wave_type = WaveType::Square;
            s.base_freq = 0.25;
            s
        }),
        ("return_fire", Sample::laser(Some(5))),
        ("grapple", {
            // A falling zip: the hook flying out on its line.
            let mut s = Sample::new();
            s.wave_type = WaveType::Square;
            s.base_freq = 0.6;
            s.freq_ramp = -0.35;
            s.env_attack = 0.0;
            s.env_sustain = 0.12;
            s.env_decay = 0.2;
            s
        }),
    ];

    let mut bufs: Vec<(String, Vec<f32>)> = sounds
        .into_iter()
        .map(|(name, sample)| {
            let mut buf = vec![0.0f32; RATE as usize * 2];
            Generator::new(sample).generate(&mut buf);
            (name.to_string(), buf)
        })
        .collect();
    // sfxr can't layer, so footsteps are synthesised here: one set per floor material, with
    // several variants each so a walk doesn't sound like the same click over and over.
    for floor in Floor::ALL {
        for i in 0..STEP_VARIANTS {
            bufs.push((format!("step_{}{i}", floor.name()), footstep(floor, i as u64)));
        }
    }
    bufs.push(("bump".into(), bump()));
    bufs.push(("whistle".into(), whistle()));
    bufs.push(("my_leg".into(), my_leg()));

    for (name, mut buf) in bufs {
        // Trim trailing silence and normalise.
        let end = buf.iter().rposition(|v| v.abs() > 1e-4).map_or(0, |i| i + 1);
        buf.truncate(end);
        let peak = buf.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
        buf.iter_mut().for_each(|v| *v *= 0.9 / peak);
        write(&dir, &name, &buf);
        // Same level as the dry one, so the crossfade only removes the highs.
        write(&dir, &format!("{name}_muffled"), &lowpass(&lowpass(&buf, 600.0), 600.0));
    }
}

pub const STEP_VARIANTS: usize = 3;

/// A vocal tract shape: three formants (Hz), how open the mouth is (0 = closed), and the
/// voice's pitch (Hz).
#[derive(Clone, Copy)]
struct Tract {
    f: [f32; 3],
    open: f32,
    pitch: f32,
}

impl Tract {
    const fn new(f1: f32, f2: f32, f3: f32, open: f32, pitch: f32) -> Self {
        Self { f: [f1, f2, f3], open, pitch }
    }

    fn lerp(self, other: Self, k: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * k;
        Self {
            f: [0, 1, 2].map(|i| mix(self.f[i], other.f[i])),
            open: mix(self.open, other.open),
            pitch: mix(self.pitch, other.pitch),
        }
    }
}

/// "MY LEG!": a strained, high, nasal yell, formant-synthesised from scratch.
///
/// The voice is additive: every harmonic of the pitch is a sine weighted by the vocal tract's
/// response at its frequency (three resonant peaks, the formants), so gliding the formants
/// between vowel targets says the word. Keyframes walk m → aa → ee ("my"), l → eh gliding into
/// a long ay ("leigh"), then the g closes it off with a silent stop and a short velar burst.
fn my_leg() -> Vec<f32> {
    // (time, tract): the formant targets are textbook adult values, raised a little for a yell.
    let keys: &[(f32, Tract)] = &[
        (0.00, Tract::new(280.0, 1100.0, 2500.0, 0.0, 330.0)),
        (0.03, Tract::new(280.0, 1100.0, 2500.0, 0.35, 360.0)), // m: nasal murmur
        (0.09, Tract::new(280.0, 1100.0, 2500.0, 0.35, 380.0)),
        (0.15, Tract::new(850.0, 1250.0, 2650.0, 1.0, 430.0)), // aa
        (0.27, Tract::new(850.0, 1300.0, 2650.0, 1.0, 470.0)),
        (0.38, Tract::new(320.0, 2300.0, 3000.0, 0.8, 500.0)), // ee
        (0.43, Tract::new(360.0, 1050.0, 2700.0, 0.45, 520.0)), // l
        (0.48, Tract::new(360.0, 1050.0, 2700.0, 0.45, 560.0)),
        (0.56, Tract::new(620.0, 1850.0, 2650.0, 1.0, 640.0)), // eh, cracking up high
        (0.78, Tract::new(500.0, 2000.0, 2700.0, 1.0, 690.0)), // gliding into the ay of "leigh"
        (1.08, Tract::new(380.0, 2250.0, 2850.0, 1.0, 680.0)),
        (1.22, Tract::new(360.0, 2280.0, 2850.0, 0.9, 610.0)),
        (1.30, Tract::new(300.0, 2200.0, 2450.0, 0.0, 520.0)), // g: velar pinch, then closure
    ];
    const BURST_AT: f32 = 1.36;
    const LEN: f32 = 1.5;

    let mut rng = StdRng::seed_from_u64(1337);
    let n = (RATE as f32 * LEN) as usize;
    let dt = 1.0 / RATE as f32;
    let at = |t: f32| -> Tract {
        let i = keys.iter().rposition(|(kt, _)| *kt <= t).unwrap_or(0);
        match keys.get(i + 1) {
            Some(&(t1, next)) => keys[i].1.lerp(next, ((t - keys[i].0) / (t1 - keys[i].0)).clamp(0.0, 1.0)),
            None => Tract { open: 0.0, ..keys[i].1 },
        }
    };
    // Bandwidths widen with the formant, as in real voices; a yell's are a bit wider still.
    let bandwidth = [110.0, 140.0, 200.0];
    let gain = [1.0, 0.7, 0.35];
    let response = |tract: &Tract, freq: f32| -> f32 {
        (0..3)
            .map(|i| {
                let x = (freq - tract.f[i]) / (bandwidth[i] / 2.0);
                gain[i] / (1.0 + x * x)
            })
            .sum()
    };

    let breath: Vec<f32> = lowpass(&highpass(&(0..n).map(|_| rng.random_range(-1.0..1.0)).collect::<Vec<f32>>(), 900.0), 6000.0);
    let mut phase = 0.0f32;
    let mut jitter = 0.0f32;
    let mut out = vec![0.0f32; n];
    for (k, v) in out.iter_mut().enumerate() {
        let t = k as f32 * dt;
        let tract = at(t);
        // Vibrato that grows as the note is held, plus a little random wobble (strain).
        jitter += (rng.random_range(-1.0..1.0) - jitter) * 0.002;
        let vibrato = 1.0 + 0.035 * (t * 6.5 * std::f32::consts::TAU).sin() * (t / 0.8).min(1.0) + 0.02 * jitter;
        let pitch = tract.pitch * vibrato;
        phase = (phase + pitch * dt).fract();
        let mut voice = 0.0;
        let mut h = 1;
        while (h as f32) * pitch < 7000.0 {
            let freq = h as f32 * pitch;
            // A bright, pressed source: harmonics fall off slower than a relaxed voice's.
            voice += (std::f32::consts::TAU * phase * h as f32).sin() * response(&tract, freq) / (h as f32).powf(0.7);
            h += 1;
        }
        // Breath rides on the second and third formants.
        let air = breath[k] * 0.08 * (response(&tract, tract.f[1]) + response(&tract, tract.f[2]));
        *v = (voice + air) * tract.open;
    }
    // Rasp: drive the voice into a soft clip, as a yell does to vocal folds.
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    out.iter_mut().for_each(|v| *v = (2.2 * *v / peak).tanh());

    // The g's release: a short burst of noise around the velar pinch.
    let burst = lowpass(&highpass(&(0..n).map(|_| rng.random_range(-1.0..1.0)).collect::<Vec<f32>>(), 1500.0), 3500.0);
    let start = (BURST_AT * RATE as f32) as usize;
    for k in start..n {
        let t = (k - start) as f32 * dt;
        out[k] += burst[k] * 1.6 * (t / 0.002).min(1.0) * (-t / 0.018).exp();
    }
    out
}

/// Every step is a heel thump (falling sine) plus a material layer:
/// - grass: soft, dull swish; mostly thump.
/// - gravel: a crunchy spray of tiny clicks.
/// - metal: a hollow clang (a few inharmonic partials ringing out).
/// - wood: a short, knocky resonance.
fn footstep(floor: Floor, seed: u64) -> Vec<f32> {
    let mut rng = StdRng::seed_from_u64(seed * 31 + floor as u64);
    let len = (RATE as f32 * 0.35) as usize;
    let dt = 1.0 / RATE as f32;
    let white: Vec<f32> = (0..len).map(|_| rng.random_range(-1.0..1.0)).collect();
    let env = |t: f32, attack: f32, decay: f32| (t / attack).min(1.0) * (-t / decay).exp();

    // Heel thump.
    let f0 = rng.random_range(85.0..115.0);
    let mut phase = 0.0f32;
    let mut out: Vec<f32> = (0..len)
        .map(|n| {
            let t = n as f32 * dt;
            phase += std::f32::consts::TAU * (f0 * (-t / 0.05).exp() + 45.0) * dt;
            phase.sin() * env(t, 0.002, 0.03)
        })
        .collect();
    let thump_gain = match floor {
        Floor::Grass => 0.9,
        Floor::Gravel => 0.5,
        Floor::Metal => 0.5,
        Floor::Wood => 0.8,
    };
    out.iter_mut().for_each(|v| *v *= thump_gain);

    match floor {
        Floor::Grass => {
            let swish = lowpass(&highpass(&white, 300.0), 1800.0);
            for (n, v) in out.iter_mut().enumerate() {
                *v += swish[n] * env(n as f32 * dt, 0.015, 0.06) * 1.2;
            }
        }
        Floor::Gravel => {
            let grit = highpass(&white, 1500.0);
            for _ in 0..rng.random_range(25..40) {
                let at = (rng.random_range(0.0f32..1.0).powi(2) * 0.12 * RATE as f32) as usize;
                let gain = rng.random_range(0.3..1.0);
                for k in 0..(0.004 * RATE as f32) as usize {
                    if at + k < len {
                        out[at + k] += grit[at + k] * gain * (-(k as f32 * dt) / 0.0012).exp();
                    }
                }
            }
        }
        Floor::Metal => {
            let base = rng.random_range(380.0..460.0);
            let partials = [(1.0, 0.5, 0.18), (2.76, 0.35, 0.12), (5.4, 0.2, 0.07), (8.9, 0.1, 0.04)];
            for (n, v) in out.iter_mut().enumerate() {
                let t = n as f32 * dt;
                for (ratio, gain, decay) in partials {
                    *v += (std::f32::consts::TAU * base * ratio * t).sin() * gain * env(t, 0.001, decay);
                }
            }
            let tick = highpass(&white, 2500.0);
            for (n, v) in out.iter_mut().enumerate() {
                *v += tick[n] * env(n as f32 * dt, 0.0005, 0.006) * 0.6;
            }
        }
        Floor::Wood => {
            let f = rng.random_range(210.0..260.0);
            let knock = lowpass(&highpass(&white, 400.0), 2500.0);
            for (n, v) in out.iter_mut().enumerate() {
                let t = n as f32 * dt;
                *v += (std::f32::consts::TAU * f * t).sin() * env(t, 0.001, 0.035) * 0.7
                    + (std::f32::consts::TAU * f * 2.3 * t).sin() * env(t, 0.001, 0.015) * 0.3
                    + knock[n] * env(t, 0.001, 0.012) * 0.6;
            }
        }
    }
    out
}

/// Walking into a wall: a heavy, dull body thud.
fn bump() -> Vec<f32> {
    let mut rng = StdRng::seed_from_u64(99);
    let len = (RATE as f32 * 0.4) as usize;
    let dt = 1.0 / RATE as f32;
    let white: Vec<f32> = (0..len).map(|_| rng.random_range(-1.0..1.0)).collect();
    let body = lowpass(&lowpass(&white, 900.0), 900.0);
    let mut phase = 0.0f32;
    (0..len)
        .map(|n| {
            let t = n as f32 * dt;
            phase += std::f32::consts::TAU * (90.0 * (-t / 0.04).exp() + 50.0) * dt;
            let a = (t / 0.002).min(1.0);
            phase.sin() * a * (-t / 0.09).exp() + body[n] * a * (-t / 0.03).exp() * 4.0
        })
        .collect()
}

/// A two-note "wheet-whoo" whistle. Bright (2-3 kHz) with a little breath, which is the
/// range ears place best, so you can tell where you are from it.
fn whistle() -> Vec<f32> {
    let mut rng = StdRng::seed_from_u64(7);
    let len = (RATE as f32 * 0.55) as usize;
    let dt = 1.0 / RATE as f32;
    let breath = highpass(&(0..len).map(|_| rng.random_range(-1.0f32..1.0)).collect::<Vec<_>>(), 1500.0);
    let mut phase = 0.0f32;
    (0..len)
        .map(|n| {
            let t = n as f32 * dt;
            // Rise to the first note, then drop to the second.
            let f = if t < 0.25 { 1800.0 + 900.0 * (t / 0.08).min(1.0) } else { 2100.0 - 300.0 * ((t - 0.25) / 0.1).min(1.0) };
            let vibrato = 1.0 + 0.01 * (std::f32::consts::TAU * 6.0 * t).sin();
            phase += std::f32::consts::TAU * f * vibrato * dt;
            let gap = if (0.22..0.28).contains(&t) { 0.3 } else { 1.0 };
            let env = (t / 0.02).min(1.0) * ((0.55 - t) / 0.08).clamp(0.0, 1.0) * gap;
            (phase.sin() + breath[n] * 0.08) * env
        })
        .collect()
}

/// One-pole low-pass.
fn lowpass(x: &[f32], cutoff: f32) -> Vec<f32> {
    let a = 1.0 - (-std::f32::consts::TAU * cutoff / RATE as f32).exp();
    let mut y = 0.0;
    x.iter()
        .map(|v| {
            y += a * (v - y);
            y
        })
        .collect()
}

fn highpass(x: &[f32], cutoff: f32) -> Vec<f32> {
    x.iter().zip(lowpass(x, cutoff)).map(|(v, l)| v - l).collect()
}

fn write(dir: &std::path::Path, name: &str, buf: &[f32]) {
    let path = dir.join(format!("{name}.wav"));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).unwrap();
    for v in buf {
        w.write_sample((v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
    }
    w.finalize().unwrap();
    println!("{} ({:.2}s)", path.display(), buf.len() as f32 / RATE as f32);
}
