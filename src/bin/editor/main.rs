//! The music editor: `cargo run --bin editor`.
//!
//! Plays a track (`music/*.score`) through the same score engine the game uses, with the
//! game simulated on the left: his suspicion, engaging, the win and the loss, or a scripted
//! round. Edit the score as a step grid per cue, the patches with sliders, or the whole file
//! as text; every edit that compiles is swapped into the running engine without stopping.
//! Save writes `music/<track>.score`.
//!
//! Options (for scripts and screenshots):
//! `--track <name>`, `--view score|patches|text`, `--cue <name>`, `--patch <name>`, `--play`,
//! `--round` (play the scripted round), `--suspicion <0..1>`, `--engaged`, `--select <stem>:<step>`,
//! `--size <w>x<h>`, `--screenshot <file.png>` (after `--wait <secs>`, then quit),
//! `--check` (no window: parse and reformat every track, then quit; `--write` saves the
//! reformatted text). `HIM_AUDIO=headless` runs without a sound card.

mod app;
mod audio;
mod ui;

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::WindowResolution;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use second_person::music::score::Score;

use app::{Editor, View, list_tracks, repo_root, track_path};

#[derive(Resource, Debug, Clone, Default)]
struct Args {
    track: Option<String>,
    view: Option<View>,
    cue: Option<String>,
    patch: Option<String>,
    play: bool,
    round: bool,
    suspicion: Option<f32>,
    engaged: bool,
    select: Option<(String, usize)>,
    size: (u32, u32),
    screenshot: Option<PathBuf>,
    wait: f32,
    check: bool,
    write: bool,
}

fn args() -> Result<Args, String> {
    let mut a = Args { size: (1500, 900), wait: 3.0, ..Args::default() };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--track" => a.track = Some(val()?),
            "--view" => a.view = Some(View::parse(&val()?).ok_or("--view score|patches|text")?),
            "--cue" => a.cue = Some(val()?),
            "--patch" => a.patch = Some(val()?),
            "--play" => a.play = true,
            "--round" => a.round = true,
            "--engaged" => a.engaged = true,
            "--suspicion" => a.suspicion = Some(val()?.parse().map_err(|e: std::num::ParseFloatError| e.to_string())?),
            "--select" => {
                let v = val()?;
                let (s, n) = v.rsplit_once(':').ok_or("--select <stem>:<step>")?;
                a.select = Some((s.to_string(), n.parse().map_err(|_| "--select <stem>:<step>")?));
            }
            "--size" => {
                let v = val()?;
                let (w, h) = v.split_once('x').ok_or("--size WxH")?;
                a.size = (w.parse().map_err(|_| "--size WxH")?, h.parse().map_err(|_| "--size WxH")?);
            }
            "--screenshot" => a.screenshot = Some(PathBuf::from(val()?)),
            "--wait" => a.wait = val()?.parse().map_err(|e: std::num::ParseFloatError| e.to_string())?,
            "--check" => a.check = true,
            "--write" => a.write = true,
            "--help" | "-h" => {
                println!("{}", include_str!("main.rs").lines().take_while(|l| l.starts_with("//!")).map(|l| l.trim_start_matches("//!").trim_start()).collect::<Vec<_>>().join("\n"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other} (try --help)")),
        }
    }
    Ok(a)
}

/// `--check`: every track through the parser and the formatter, no window.
fn check(write: bool) -> Result<String, String> {
    let root = repo_root();
    let tracks = list_tracks(&root);
    for t in &tracks {
        let path = track_path(&root, t);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{t}: {e}"))?;
        let score = Score::parse(&text).map_err(|e| format!("music/{t}.score: {e}"))?;
        let again = Score::parse(&score.to_text()).map_err(|e| format!("music/{t}.score reformatted: {e}"))?;
        if again != score {
            return Err(format!("music/{t}.score doesn't survive a reformat"));
        }
        if write {
            std::fs::write(&path, score.to_text()).map_err(|e| e.to_string())?;
        }
    }
    Ok(format!("{} tracks ok", tracks.len()))
}

fn main() {
    let a = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("editor: {e}");
            std::process::exit(2);
        }
    };
    if a.check {
        match check(a.write) {
            Ok(m) => println!("{m}"),
            Err(e) => {
                eprintln!("editor: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let mut ed = match Editor::new(repo_root(), a.track.as_deref()) {
        Ok(ed) => ed,
        Err(e) => {
            eprintln!("editor: {e}");
            std::process::exit(1);
        }
    };
    if let Some(v) = a.view {
        ed.view = v;
    }
    if let Some(c) = &a.cue {
        ed.cue = ed.doc.cues.iter().position(|x| &x.name == c).unwrap_or(0);
    }
    if a.patch.is_some() {
        ed.patch = a.patch.clone();
    }
    if let Some(s) = a.suspicion {
        ed.suspicion = s;
    }
    if let Some((stem, step)) = &a.select
        && let Some(i) = ed.doc.stems.iter().position(|s| &s.name == stem)
    {
        ed.sel = Some((i, *step));
        ed.cell = second_person::music::pattern::tokens(&ed.doc.stems[i].part).ok().and_then(|t| t.get(*step).cloned()).unwrap_or_default();
    }
    App::new()
        .insert_resource(ClearColor(Color::srgb_u8(0x0c, 0x12, 0x14)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "HIM · music editor".into(),
                resolution: WindowResolution::from(a.size),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_non_send(ed)
        .insert_resource(a)
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Camera2d);
        })
        .add_systems(EguiPrimaryContextPass, draw)
        .add_systems(Update, script)
        .run();
}

fn draw(mut contexts: EguiContexts, mut ed: NonSendMut<Editor>, time: Res<Time<Real>>, mut styled: Local<bool>) -> Result {
    let ctx = contexts.ctx_mut()?;
    if !*styled {
        ui::style(ctx);
        *styled = true;
    }
    ed.update(time.elapsed_secs_f64());
    ui::frame(&mut ed, ctx);
    Ok(())
}

/// The command line's script: start playing, screenshot, quit.
fn script(mut commands: Commands, time: Res<Time<Real>>, args: Res<Args>, mut ed: NonSendMut<Editor>, mut stage: Local<u8>, mut exit: MessageWriter<AppExit>) {
    let t = time.elapsed_secs();
    if *stage == 0 && t > 0.3 {
        *stage = 1;
        if args.round {
            ed.play_round();
        } else if args.play {
            match args.cue.as_ref().and_then(|c| ed.doc.cues.iter().position(|x| &x.name == c)) {
                Some(i) if i != 0 => ed.play_cue(i),
                _ => ed.play(),
            }
        }
        if args.engaged {
            ed.set_engaged(true);
        }
    }
    if let Some(path) = &args.screenshot {
        if *stage == 1 && t > args.wait {
            *stage = 2;
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
        }
        if *stage == 2 && t > args.wait + 1.5 {
            exit.write(AppExit::Success);
        }
    }
}
