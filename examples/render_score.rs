//! Render a track's scripted round to a WAV, the way the game would play it:
//! `cargo run --release --example render_score -- [track] [out.wav]`.

use second_person::music::score::Score;
use second_person::music::{self, Engine, script};

fn main() {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "him".into());
    let out = args.next().unwrap_or_else(|| format!("{name}.wav"));
    let path = format!("{}/music/{name}.score", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let score = Score::parse(&text).unwrap_or_else(|e| panic!("{path}: {e}"));
    const SR: u32 = 48000;
    let mut e = Engine::new(score.compile().unwrap(), SR as f64);
    e.set_soundfont(Some(music::load_soundfont(format!("{}/{}", env!("CARGO_MANIFEST_DIR"), music::SOUNDFONT)).unwrap()));
    e.play();
    let spec = hound::WavSpec { channels: 2, sample_rate: SR, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(&out, spec).unwrap();
    // 10 ms of game per update, like a frame or so.
    let mut buf = vec![0.0f32; (SR / 100) as usize * 2];
    let mut won = false;
    let mut t = 0.0f32;
    let mut last = String::new();
    while t < script::LENGTH + 1.0 {
        let m = script::at(t);
        e.suspicion = m.suspicion;
        e.engaged = m.engaged;
        if m.won && !won {
            won = true;
            e.end(true);
        }
        e.render(&mut buf);
        for s in &buf {
            w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
        }
        let s = e.status();
        let now = format!("{} bar {}", s.cue, s.bar + 1);
        if now != last {
            println!("{t:6.2}s  {now}  suspicion {:.2}", m.suspicion);
            last = now;
        }
        t += 0.01;
    }
    w.finalize().unwrap();
    println!("wrote {out}");
}
