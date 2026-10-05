//! Regenerates the procedural sound effects in `assets/sfx/` with sfxr.
//!
//!     cargo run --example gen_sfx
//!
//! Seeds are fixed so the output is reproducible; tweak a seed or parameter and rerun.

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
        ("step", {
            let mut s = Sample::new();
            s.wave_type = WaveType::Noise;
            s.base_freq = 0.12;
            s.env_attack = 0.0;
            s.env_sustain = 0.02;
            s.env_decay = 0.08;
            s.lpf_freq = 0.25;
            s
        }),
        ("ping", {
            let mut s = Sample::blip(Some(2));
            s.wave_type = WaveType::Sine;
            s.base_freq = 0.55;
            s.env_decay = 0.35;
            s
        }),
    ];

    for (name, sample) in sounds {
        let mut buf = vec![0.0f32; RATE as usize * 2];
        Generator::new(sample).generate(&mut buf);
        // Trim trailing silence and normalise.
        let end = buf.iter().rposition(|v| v.abs() > 1e-4).map_or(0, |i| i + 1);
        buf.truncate(end);
        let peak = buf.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);

        let path = dir.join(format!("{name}.wav"));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for v in &buf {
            w.write_sample((v / peak * 0.9 * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
        println!("{} ({:.2}s)", path.display(), buf.len() as f32 / RATE as f32);
    }
}
