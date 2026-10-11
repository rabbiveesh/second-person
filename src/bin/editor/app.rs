//! The editor's state: the open track (as a `Score` and as text), the views' selections, and
//! the simulated game that drives the engine.

use std::path::{Path, PathBuf};

use second_person::music::score::Score;
use second_person::music::{self, Status, script};

use crate::audio::{self, Output};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Score,
    Patches,
    Text,
}

impl View {
    pub fn parse(s: &str) -> Option<View> {
        match s {
            "score" => Some(View::Score),
            "patches" => Some(View::Patches),
            "text" => Some(View::Text),
            _ => None,
        }
    }
}

pub struct Editor {
    pub root: PathBuf,
    pub tracks: Vec<String>,
    pub track: String,
    pub doc: Score,
    /// The text view's buffer: the doc's text, or the user's edit in progress.
    pub text: String,
    pub text_error: Option<String>,
    saved: String,
    pub view: View,
    /// The cue the score view shows.
    pub cue: usize,
    /// Selected cell: (stem index, step).
    pub sel: Option<(usize, usize)>,
    pub cell: String,
    pub cell_error: Option<String>,
    pub patch: Option<String>,
    pub new_name: String,
    pub out: Output,
    pub status: Status,
    /// The scripted round: when it started (editor clock).
    pub round: Option<f64>,
    pub suspicion: f32,
    pub engaged: bool,
    pub ended: Option<bool>,
    pub toast: Option<(String, f64)>,
    pub now: f64,
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn list_tracks(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root.join(music::DIR))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".score")).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

pub fn track_path(root: &Path, name: &str) -> PathBuf {
    root.join(music::DIR).join(format!("{name}.score"))
}

impl Editor {
    pub fn new(root: PathBuf, track: Option<&str>) -> Result<Editor, String> {
        let tracks = list_tracks(&root);
        let name = track.map(String::from).or_else(|| tracks.first().cloned()).ok_or("no tracks in music/")?;
        let text = std::fs::read_to_string(track_path(&root, &name)).map_err(|e| format!("{name}: {e}"))?;
        let doc = Score::parse(&text).map_err(|e| format!("{name}.score: {e}"))?;
        let out = audio::start(doc.compile()?);
        let status = out.engine.lock().unwrap().status();
        Ok(Editor {
            root,
            tracks,
            track: name,
            text: text.clone(),
            text_error: None,
            saved: text,
            doc,
            view: View::Score,
            cue: 0,
            sel: None,
            cell: String::new(),
            cell_error: None,
            patch: None,
            new_name: String::new(),
            out,
            status,
            round: None,
            suspicion: 0.0,
            engaged: false,
            ended: None,
            toast: None,
            now: 0.0,
        })
    }

    pub fn dirty(&self) -> bool {
        self.text != self.saved
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), self.now + 3.0));
    }

    /// Apply a structural edit: the doc changed; refresh the text and the engine.
    pub fn commit(&mut self) {
        match self.doc.compile() {
            Ok(c) => {
                self.out.engine.lock().unwrap().set_score(c);
                self.text = self.doc.to_text();
                self.text_error = None;
            }
            Err(e) => self.say(format!("not applied: {e}")),
        }
    }

    /// The text view changed: take it if it parses.
    pub fn set_text(&mut self, text: String) {
        self.text = text;
        match Score::parse(&self.text) {
            Ok(doc) => {
                let c = doc.compile().expect("parse compiles");
                self.doc = doc;
                self.out.engine.lock().unwrap().set_score(c);
                self.text_error = None;
                self.cue = self.cue.min(self.doc.cues.len().saturating_sub(1));
            }
            Err(e) => self.text_error = Some(e),
        }
    }

    pub fn save(&mut self) {
        if self.text_error.is_some() {
            self.say("fix the text first: it doesn't parse");
            return;
        }
        // From the score view the text is already canonical; from the text view keep the
        // user's own layout and comments.
        let path = track_path(&self.root, &self.track);
        match std::fs::write(&path, &self.text) {
            Ok(()) => {
                self.saved = self.text.clone();
                self.say(format!("saved music/{}.score", self.track));
            }
            Err(e) => self.say(format!("save failed: {e}")),
        }
    }

    pub fn open(&mut self, name: &str) {
        let Ok(text) = std::fs::read_to_string(track_path(&self.root, name)) else {
            self.say(format!("can't read {name}"));
            return;
        };
        match Score::parse(&text) {
            Ok(doc) => {
                let c = doc.compile().expect("parse compiles");
                self.track = name.to_string();
                self.doc = doc;
                self.text = text.clone();
                self.saved = text;
                self.text_error = None;
                self.cue = 0;
                self.sel = None;
                self.patch = None;
                self.out.engine.lock().unwrap().set_score(c);
            }
            Err(e) => self.say(format!("{name}.score: {e}")),
        }
    }

    /// Save the current track under a new name and switch to it.
    pub fn save_as(&mut self, name: &str) {
        let name = name.trim().to_lowercase().replace(' ', "_");
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            self.say("name it with letters, digits, - or _");
            return;
        }
        if self.tracks.contains(&name) {
            self.say(format!("{name} already exists"));
            return;
        }
        self.track = name.clone();
        self.doc.title = name.replace('_', " ");
        self.commit();
        self.save();
        self.tracks = list_tracks(&self.root);
    }

    // ── Transport and the simulated game ──

    pub fn play(&mut self) {
        self.round = None;
        self.ended = None;
        self.engaged = false;
        let mut e = self.out.engine.lock().unwrap();
        e.engaged = false;
        e.play();
    }

    pub fn play_cue(&mut self, cue: usize) {
        self.round = None;
        self.ended = None;
        self.out.engine.lock().unwrap().play_cue(cue);
    }

    pub fn stop(&mut self) {
        self.round = None;
        self.out.engine.lock().unwrap().stop();
    }

    pub fn toggle_play(&mut self) {
        if self.status.playing { self.stop() } else { self.play() }
    }

    pub fn play_round(&mut self) {
        self.play();
        self.round = Some(self.now);
    }

    pub fn set_engaged(&mut self, on: bool) {
        self.round = None;
        self.engaged = on;
        if on {
            self.suspicion = 1.0;
        }
    }

    pub fn end(&mut self, won: bool) {
        self.round = None;
        self.ended = Some(won);
        self.engaged = false;
        let mut e = self.out.engine.lock().unwrap();
        if !e.is_playing() {
            e.play();
        }
        e.end(won);
    }

    /// Once a frame: run the scripted round, push the game state, read the meters.
    pub fn update(&mut self, now: f64) {
        self.now = now;
        if let Some(start) = self.round {
            let t = (now - start) as f32;
            let m = script::at(t);
            self.suspicion = m.suspicion;
            self.engaged = m.engaged;
            if m.won && self.ended.is_none() {
                self.ended = Some(true);
                self.out.engine.lock().unwrap().end(true);
            }
            if t > script::LENGTH {
                self.round = None;
            }
        }
        let mut e = self.out.engine.lock().unwrap();
        e.suspicion = self.suspicion;
        e.engaged = self.engaged;
        self.status = e.status();
        drop(e);
        if self.toast.as_ref().is_some_and(|(_, until)| now > *until) {
            self.toast = None;
        }
    }

    /// His state in words, as the game's HUD would put it.
    pub fn caption(&self) -> &'static str {
        match self.ended {
            Some(true) => "He's down",
            Some(false) => "He got you",
            None => script::caption(script::Moment { suspicion: self.suspicion, engaged: self.engaged, won: false }),
        }
    }

    pub fn audition_patch(&mut self, name: &str, notes: &[i32], gate: f64) {
        if let Some(p) = self.doc.patches.get(name) {
            let p = p.clone();
            self.out.engine.lock().unwrap().audition(&p, notes, gate);
        }
    }
}
