//! The conductor: turns a score and the game's state into sound, a block at a time.
//!
//! Every stem of every cue has its own bus (a fundsp `Sequencer` of notes) and fader. Notes
//! are scheduled sample-accurately just before the block that plays them, so the music
//! answers the game within one block (~1.3 ms) while staying on the grid:
//! - **Vertical layering**: in `calm`, his suspicion moves each stem's fader along its rule.
//! - **Horizontal resequencing**: engaging (or letting go) switches cue on the next bar line,
//!   from the top of the new cue, at its tempo.
//! - **Stingers**: `end` plays `win`/`lose` once, from the next beat.
//!
//! Stems whose patch names a soundfont `sample` play through their own `rustysynth`
//! synthesizer once `set_soundfont` gives the engine a bank; until then they use the synth.
//!
//! Game-free and Bevy-free: the game, the editor and offline renders drive it the same way.

use std::collections::HashSet;
use std::sync::Arc;

use fundsp::prelude32::{
    AudioUnit, BufferRef, BufferVec, Fade, MAX_BUFFER_SIZE, ReplayMode, Sequencer, limiter_stereo, lowpole_hz, reverb2_stereo, zero,
};

use super::pattern::{self, STEPS_PER_BAR, Step};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

use super::score::{Compiled, CompiledSample, Patch, Role};
use super::voice;

/// GM's percussion channel.
const DRUMS: i32 = 9;

struct Bus {
    name: String,
    seq: Sequencer,
    /// Set when the stem plays from the soundfont.
    sampler: Option<Sampler>,
    gain: f32,
    /// Peak follower of what the bus actually outputs, for meters.
    meter: f32,
}

impl Bus {
    fn new(name: &str, sr: f64) -> Bus {
        let mut seq = Sequencer::new(0, 2, ReplayMode::None);
        seq.set_sample_rate(sr);
        Bus { name: name.to_string(), seq, sampler: None, gain: 0.0, meter: 0.0 }
    }
}

/// One stem's soundfont instrument: a synthesizer of its own (so the stem keeps its own
/// fader and meter) and the note events waiting for their sample.
struct Sampler {
    synth: Synthesizer,
    program: Option<u8>,
    gain: f32,
    /// (engine time, key, velocity; 0 = note off), in time order.
    events: Vec<(f64, i32, i32)>,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Sampler {
    fn new(font: &Arc<SoundFont>, sample: &CompiledSample, sr: f64) -> Option<Sampler> {
        let mut settings = SynthesizerSettings::new(sr as i32);
        // The engine's own reverb and limiter finish the mix.
        settings.enable_reverb_and_chorus = false;
        settings.maximum_polyphony = 32;
        let mut synth = Synthesizer::new(font, &settings).ok()?;
        if let Some(p) = sample.program {
            synth.process_midi_message(0, 0xC0, p as i32, 0);
        }
        Some(Sampler { synth, program: sample.program, gain: sample.gain, events: Vec::new(), left: Vec::new(), right: Vec::new() })
    }

    fn channel(&self) -> i32 {
        if self.program.is_some() { 0 } else { DRUMS }
    }

    fn push(&mut self, at: f64, key: i32, vel: i32) {
        let i = self.events.partition_point(|e| e.0 <= at);
        self.events.insert(i, (at, key, vel));
    }

    /// Render `n` samples from engine time `t` into `out`, firing events on their sample.
    fn render(&mut self, t: f64, n: usize, sr: f64, out: &mut BufferVec) {
        let mut done = 0;
        while done < n {
            let now = t + done as f64 / sr;
            let ch = self.channel();
            while let Some(&(at, key, vel)) = self.events.first() {
                if at > now {
                    break;
                }
                self.events.remove(0);
                if vel > 0 {
                    self.synth.note_on(ch, key, vel);
                } else {
                    self.synth.note_off(ch, key);
                }
            }
            let until = self.events.first().map_or(n, |e| (((e.0 - t) * sr).ceil() as usize).clamp(done + 1, n));
            let len = until - done;
            self.left.resize(len, 0.0);
            self.right.resize(len, 0.0);
            self.synth.render(&mut self.left, &mut self.right);
            for k in 0..len {
                out.set_f32(0, done + k, self.left[k] * self.gain);
                out.set_f32(1, done + k, self.right[k] * self.gain);
            }
            done = until;
        }
    }
}

/// What a stem is doing right now, for meters.
#[derive(Clone, Debug, Default)]
pub struct StemStatus {
    pub name: String,
    pub cue: String,
    /// Fader position 0..1 (after mute/solo).
    pub gain: f32,
    /// Output peak, decaying.
    pub meter: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub playing: bool,
    pub cue: String,
    /// Position of the step now sounding, 0-based.
    pub bar: usize,
    pub step: usize,
    pub bpm: f32,
    /// A switch or stinger that waits for the bar line / next beat.
    pub pending: Option<String>,
    pub stems: Vec<StemStatus>,
}

pub struct Engine {
    sr: f64,
    score: Compiled,
    buses: Vec<Bus>,
    audition: Bus,
    reverb: Box<dyn AudioUnit>,
    limiter: Box<dyn AudioUnit>,
    soundfont: Option<Arc<SoundFont>>,
    time: f64,

    playing: bool,
    cue: usize,
    step: usize,
    next_at: f64,
    ending: Option<bool>,
    /// Times the current cue has looped since it started (`once` stems play on pass 0).
    pass: usize,
    /// The stinger has played out: nothing more is scheduled.
    done: bool,

    pub suspicion: f32,
    pub engaged: bool,
    pub muted: HashSet<String>,
    pub soloed: HashSet<String>,

    dry: BufferVec,
    send: BufferVec,
    wet: BufferVec,
    bus_out: BufferVec,
}

const BLOCK: usize = MAX_BUFFER_SIZE;

impl Engine {
    pub fn new(score: Compiled, sample_rate: f64) -> Engine {
        let mut e = Engine {
            sr: sample_rate,
            buses: Vec::new(),
            audition: Bus::new("audition", sample_rate),
            reverb: Box::new(zero()),
            limiter: Box::new(limiter_stereo(0.004, 0.25)),
            soundfont: None,
            score,
            time: 0.0,
            playing: false,
            cue: 0,
            step: 0,
            next_at: 0.0,
            ending: None,
            pass: 0,
            done: false,
            suspicion: 0.0,
            engaged: false,
            muted: HashSet::new(),
            soloed: HashSet::new(),
            dry: BufferVec::new(2),
            send: BufferVec::new(2),
            wet: BufferVec::new(2),
            bus_out: BufferVec::new(2),
        };
        e.audition.gain = 1.0;
        e.limiter.set_sample_rate(sample_rate);
        e.limiter.allocate();
        e.rebuild();
        e.cue = e.score.cue(Role::Calm).unwrap_or(0);
        e
    }

    pub fn sample_rate(&self) -> f64 {
        self.sr
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    /// Give the engine a soundfont (or take it away): stems with a `sample` play from it.
    pub fn set_soundfont(&mut self, font: Option<Arc<SoundFont>>) {
        self.soundfont = font;
        for b in &mut self.buses {
            b.sampler = None;
        }
        self.attach_samplers();
    }

    pub fn has_soundfont(&self) -> bool {
        self.soundfont.is_some()
    }

    /// Make sure every sampled stem has a sampler for its current instrument.
    fn attach_samplers(&mut self) {
        let Some(font) = &self.soundfont else { return };
        for (s, b) in self.score.stems.iter().zip(&mut self.buses) {
            match &s.sample {
                None => b.sampler = None,
                Some(x) => match &mut b.sampler {
                    Some(sm) if sm.program == x.program => sm.gain = x.gain,
                    _ => b.sampler = Sampler::new(font, x, self.sr),
                },
            }
        }
    }

    /// Swap in an edited score without stopping: stems keep their buses (and ringing notes)
    /// by name, the current cue by name, and the position wraps into the new length.
    pub fn set_score(&mut self, score: Compiled) {
        let room_changed = score.room != self.score.room;
        let cue_name = self.score.cues.get(self.cue).map(|c| c.name.clone());
        self.score = score;
        if room_changed {
            self.rebuild_reverb();
        }
        let mut old: Vec<Bus> = std::mem::take(&mut self.buses);
        for s in &self.score.stems {
            let bus = match old.iter().position(|b| b.name == s.name) {
                Some(i) => old.swap_remove(i),
                None => Bus::new(&s.name, self.sr),
            };
            self.buses.push(bus);
        }
        self.attach_samplers();
        self.cue = cue_name
            .and_then(|n| self.score.cues.iter().position(|c| c.name == n))
            .or_else(|| self.score.cue(Role::Calm))
            .unwrap_or(0);
        self.step %= self.score.cues[self.cue].steps();
    }

    fn rebuild(&mut self) {
        self.buses = self.score.stems.iter().map(|s| Bus::new(&s.name, self.sr)).collect();
        self.attach_samplers();
        self.rebuild_reverb();
    }

    fn rebuild_reverb(&mut self) {
        let mut r = reverb2_stereo(18.0, self.score.room.clamp(0.2, 12.0), 0.8, 0.3, lowpole_hz(5000.0));
        r.set_sample_rate(self.sr);
        r.allocate();
        self.reverb = Box::new(r);
    }

    /// Start the round's music from the top of `calm`.
    pub fn play(&mut self) {
        self.play_cue(self.score.cue(Role::Calm).unwrap_or(0));
    }

    /// Start from the top of any cue (the editor's cue tabs). A loop cue then follows the
    /// game state as usual; a stinger plays once.
    pub fn play_cue(&mut self, cue: usize) {
        self.cue = cue.min(self.score.cues.len() - 1);
        self.step = 0;
        self.next_at = self.time + 0.03;
        self.playing = true;
        self.ending = None;
        self.pass = 0;
        self.done = false;
    }

    pub fn stop(&mut self) {
        self.playing = false;
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// The round is over: play the win or lose stinger from the next beat.
    pub fn end(&mut self, won: bool) {
        if self.playing && !self.done && !self.score.cues[self.cue].role.once() {
            self.ending = Some(won);
        }
    }

    /// Play a patch right now, outside the score (patch previews).
    pub fn audition(&mut self, patch: &Patch, notes: &[i32], gate: f64) {
        for &m in notes {
            let unit = voice::note(patch, m as f32, gate, 1.0);
            self.audition.seq.push_relative(0.0, voice::length(patch, gate), Fade::Smooth, 0.0, 0.005, unit);
        }
    }

    pub fn status(&self) -> Status {
        let cue = &self.score.cues[self.cue];
        let now = (self.step + cue.steps() - 1) % cue.steps();
        let pending = if let Some(won) = self.ending {
            Some(if won { "win" } else { "lose" }.to_string())
        } else if self.playing && !cue.role.once() {
            let want = self.wanted_loop();
            (want != self.cue).then(|| self.score.cues[want].name.clone())
        } else {
            None
        };
        Status {
            playing: self.playing && !self.done,
            cue: cue.name.clone(),
            bar: now / STEPS_PER_BAR,
            step: now,
            bpm: (15.0 / cue.step) as f32,
            pending,
            stems: self
                .score
                .stems
                .iter()
                .zip(&self.buses)
                .map(|(s, b)| StemStatus { name: s.name.clone(), cue: self.score.cues[s.cue].name.clone(), gain: b.gain, meter: b.meter })
                .collect(),
        }
    }

    /// The loop cue the game state asks for.
    fn wanted_loop(&self) -> usize {
        let role = if self.engaged { Role::Fight } else { Role::Calm };
        let cur = &self.score.cues[self.cue];
        if cur.role == Role::Other {
            return self.cue;
        }
        self.score.cue(role).unwrap_or(self.cue)
    }

    fn target(&self, i: usize) -> f32 {
        let s = &self.score.stems[i];
        if self.muted.contains(&s.name) || (!self.soloed.is_empty() && !self.soloed.contains(&s.name)) {
            return 0.0;
        }
        if !self.playing || s.cue != self.cue {
            return 0.0;
        }
        let cue = &self.score.cues[self.cue];
        let rule = if cue.role == Role::Calm { s.fader.at(self.suspicion) } else { 1.0 };
        s.level * rule
    }

    /// Schedule every step that starts before `until`.
    fn schedule(&mut self, until: f64) {
        while self.playing && !self.done && self.next_at < until {
            let cue = &self.score.cues[self.cue];
            if !cue.role.once() {
                if let Some(won) = self.ending.filter(|_| self.step % 4 == 0) {
                    self.ending = None;
                    match self.score.cue(if won { Role::Win } else { Role::Lose }) {
                        Some(c) => {
                            self.cue = c;
                            self.step = 0;
                            self.pass = 0;
                        }
                        None => {
                            self.done = true;
                            return;
                        }
                    }
                } else if self.step % STEPS_PER_BAR == 0 {
                    let want = self.wanted_loop();
                    if want != self.cue {
                        self.cue = want;
                        self.step = 0;
                        self.pass = 0;
                    }
                }
            }
            self.play_step(self.next_at);
            let cue = &self.score.cues[self.cue];
            self.next_at += cue.step;
            self.step += 1;
            if self.step >= cue.steps() {
                if cue.role.once() {
                    self.done = true;
                }
                self.step = 0;
                self.pass += 1;
            }
        }
    }

    fn play_step(&mut self, at: f64) {
        let cue_i = self.cue;
        let cue = &self.score.cues[cue_i];
        let chord = &cue.chords[(self.step / STEPS_PER_BAR) % cue.bars()];
        let rel = (at - self.time).max(0.0);
        for (stem, bus) in self.score.stems.iter().zip(self.buses.iter_mut()) {
            if stem.cue != cue_i || (stem.once && self.pass > 0) {
                continue;
            }
            let len = stem.steps.len();
            let i = self.step % len;
            let Step::Notes(pitches, vel) = &stem.steps[i] else { continue };
            let mut held = 1;
            while held < len && stem.steps[(i + held) % len] == Step::Hold {
                held += 1;
            }
            // A stinger's notes stop at its end; a loop's may tie over its bar line.
            if cue.role.once() {
                held = held.min(cue.steps() - self.step);
            }
            let gate = held as f64 * cue.step;
            let mut notes = Vec::new();
            for p in pitches {
                // `x` plays the patch's hit pitch, itself read against the chord.
                let one = std::slice::from_ref(p);
                let src = if *p == pattern::Pitch::Base { &stem.hit[..] } else { one };
                notes.extend(pattern::resolve(src, chord, 0).into_iter().map(|m| m + self.score.transpose));
            }
            if let (Some(sm), Some(sample)) = (&mut bus.sampler, &stem.sample) {
                let v = ((96.0 * vel).round() as i32).clamp(1, 127);
                for m in notes {
                    if sample.program.is_some() {
                        sm.push(at, m, v);
                        sm.push(at + gate, m, 0);
                    } else {
                        // Drum keys are read from the written note, before transposing.
                        sm.push(at, sample.key(m - self.score.transpose) as i32, v);
                    }
                }
                continue;
            }
            for m in notes {
                let unit = voice::note(&stem.patch, m as f32, gate, *vel);
                bus.seq.push_relative(rel, rel + voice::length(&stem.patch, gate), Fade::Smooth, 0.0, 0.005, unit);
            }
        }
    }

    /// Render interleaved stereo.
    pub fn render(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(BLOCK);
            self.render_block(n);
            for i in 0..n {
                out[(done + i) * 2] = self.dry.at_f32(0, i);
                out[(done + i) * 2 + 1] = self.dry.at_f32(1, i);
            }
            done += n;
        }
    }

    fn render_block(&mut self, n: usize) {
        self.schedule(self.time + n as f64 / self.sr);
        self.dry.clear();
        self.send.clear();
        let tau = self.score.cues[self.cue].fade.max(0.005) as f64;
        let tau = if self.playing { tau } else { 0.15 };
        let coef = (1.0 - (-1.0 / (tau * self.sr)).exp()) as f32;
        let meter_fall = (-(n as f64) / (0.25 * self.sr)).exp() as f32;
        for i in 0..self.buses.len() {
            let target = self.target(i);
            let send = self.score.stems[i].patch.send;
            let bus = &mut self.buses[i];
            match &mut bus.sampler {
                Some(sm) => sm.render(self.time, n, self.sr, &mut self.bus_out),
                None => bus.seq.process(n, &BufferRef::empty(), &mut self.bus_out.buffer_mut()),
            }
            if bus.gain < 1e-5 && target == 0.0 {
                // Silent and staying silent: its clock still ran, so tails stay in time.
                bus.gain = 0.0;
                bus.meter *= meter_fall;
                continue;
            }
            let mut peak = 0.0f32;
            for k in 0..n {
                bus.gain += (target - bus.gain) * coef;
                let l = self.bus_out.at_f32(0, k) * bus.gain;
                let r = self.bus_out.at_f32(1, k) * bus.gain;
                peak = peak.max(l.abs()).max(r.abs());
                self.dry.set_f32(0, k, self.dry.at_f32(0, k) + l);
                self.dry.set_f32(1, k, self.dry.at_f32(1, k) + r);
                self.send.set_f32(0, k, self.send.at_f32(0, k) + l * send);
                self.send.set_f32(1, k, self.send.at_f32(1, k) + r * send);
            }
            bus.meter = (bus.meter * meter_fall).max(peak);
        }
        self.audition.seq.process(n, &BufferRef::empty(), &mut self.bus_out.buffer_mut());
        for k in 0..n {
            for ch in 0..2 {
                let a = self.bus_out.at_f32(ch, k);
                self.dry.set_f32(ch, k, self.dry.at_f32(ch, k) + a);
                self.send.set_f32(ch, k, self.send.at_f32(ch, k) + a);
            }
        }
        self.reverb.process(n, &self.send.buffer_ref(), &mut self.wet.buffer_mut());
        let wet = self.score.reverb.clamp(0.0, 1.0);
        let gain = self.score.gain;
        for k in 0..n {
            for ch in 0..2 {
                let v = (self.dry.at_f32(ch, k) + self.wet.at_f32(ch, k) * wet) * gain;
                self.send.set_f32(ch, k, v);
            }
        }
        self.limiter.process(n, &self.send.buffer_ref(), &mut self.dry.buffer_mut());
        self.time += n as f64 / self.sr;
    }
}
