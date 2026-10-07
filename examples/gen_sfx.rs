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
