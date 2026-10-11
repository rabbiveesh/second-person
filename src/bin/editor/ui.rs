//! Drawing: the top bar, HIM's side panel (the simulated game and the mixer), and the
//! three views of a track: the score grid, the patches, the text.

use bevy_egui::egui::{self, Align2, Color32, FontId, Key, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use second_person::music::pattern::{self, STEPS_PER_BAR};
use second_person::music::score::{Cue, Fader, FilterKind, Osc, Patch, Role, Stem, Wave};
use second_person::music::theory;

use crate::app::{Editor, View};

pub const BG: Color32 = Color32::from_rgb(0x0c, 0x12, 0x14);
pub const PANEL: Color32 = Color32::from_rgb(0x13, 0x1c, 0x1f);
pub const RAISED: Color32 = Color32::from_rgb(0x1a, 0x26, 0x2a);
pub const LINE: Color32 = Color32::from_rgb(0x24, 0x32, 0x3a);
pub const BAR_LINE: Color32 = Color32::from_rgb(0x3d, 0x52, 0x5c);
pub const FG: Color32 = Color32::from_rgb(0xdd, 0xe5, 0xe2);
pub const MUTED: Color32 = Color32::from_rgb(0x8b, 0x9a, 0x98);
pub const FAINT: Color32 = Color32::from_rgb(0x4e, 0x5d, 0x62);
pub const CALM: Color32 = Color32::from_rgb(0x4f, 0xd1, 0x8b);
pub const WARN: Color32 = Color32::from_rgb(0xe3, 0xa3, 0x4a);
pub const HOT: Color32 = Color32::from_rgb(0xe0, 0x53, 0x3f);
pub const WIN: Color32 = Color32::from_rgb(0xe8, 0xc5, 0x6a);
pub const LOSE: Color32 = Color32::from_rgb(0x9d, 0x8c, 0xc9);
pub const OTHER: Color32 = Color32::from_rgb(0x7a, 0xb8, 0xff);

pub fn cue_color(name: &str) -> Color32 {
    match Role::of(name) {
        Role::Calm => CALM,
        Role::Fight => HOT,
        Role::Win => WIN,
        Role::Lose => LOSE,
        Role::Other => OTHER,
    }
}

pub fn style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = RAISED;
    v.override_text_color = Some(FG);
    v.selection.bg_fill = Color32::from_rgb(0x2c, 0x4a, 0x40);
    v.selection.stroke = Stroke::new(1.0, CALM);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x22, 0x33, 0x38);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x2a, 0x3d, 0x43);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(8.0, 4.0);
    });
}

/// The whole window, one frame.
pub fn frame(ed: &mut Editor, ctx: &egui::Context) {
    keys(ed, ctx);
    let mut root = egui::Ui::new(ctx.clone(), "root".into(), egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()));
    root.painter().rect_filled(ctx.viewport_rect(), 0.0, BG);
    let panel = |fill| egui::Frame::new().fill(fill).inner_margin(egui::Margin::same(12));
    egui::Panel::top("top").exact_size(52.0).resizable(false).frame(panel(PANEL).inner_margin(egui::Margin::symmetric(14, 0))).show_inside(&mut root, |ui| top(ed, ui));
    egui::Panel::left("him").exact_size(290.0).resizable(false).frame(panel(PANEL)).show_inside(&mut root, |ui| him(ed, ui));
    egui::CentralPanel::default().frame(panel(BG)).show_inside(&mut root, |ui| match ed.view {
        View::Score => score_view(ed, ui),
        View::Patches => patches_view(ed, ui),
        View::Text => text_view(ed, ui),
    });
    if let Some((msg, _)) = &ed.toast {
        egui::Area::new("toast".into()).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0)).show(ctx, |ui| {
            egui::Frame::new().fill(RAISED).stroke(Stroke::new(1.0, LINE)).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                ui.label(RichText::new(msg).color(FG));
            });
        });
    }
}

/// Shortcuts: ctrl+S save; and when not typing: space play/stop, R a scripted round,
/// E engage, W win, L lose, [ and ] suspicion.
fn keys(ed: &mut Editor, ctx: &egui::Context) {
    let save = ctx.input(|i| i.modifiers.command && i.key_pressed(Key::S));
    if save {
        ed.save();
    }
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let pressed = |k| ctx.input(|i| i.key_pressed(k) && !i.modifiers.command);
    if pressed(Key::Space) {
        ed.toggle_play();
    }
    if pressed(Key::R) {
        ed.play_round();
    }
    if pressed(Key::E) {
        let on = !ed.engaged;
        ed.set_engaged(on);
    }
    if pressed(Key::W) {
        ed.end(true);
    }
    if pressed(Key::L) {
        ed.end(false);
    }
    if pressed(Key::OpenBracket) {
        ed.round = None;
        ed.suspicion = (ed.suspicion - 0.1).max(0.0);
    }
    if pressed(Key::CloseBracket) {
        ed.round = None;
        ed.suspicion = (ed.suspicion + 0.1).min(1.0);
    }
    if ed.view == View::Score {
        grid_keys(ed, ctx);
    }
}

fn top(ed: &mut Editor, ui: &mut egui::Ui) {
    ui.horizontal_centered(|ui| {
        ui.label(RichText::new("HIM").size(22.0).strong().color(FG));
        ui.label(RichText::new("MUSIC").size(13.0).color(MUTED));
        ui.add_space(12.0);
        let mut pick = None;
        egui::ComboBox::from_id_salt("track").selected_text(RichText::new(&ed.track).strong()).width(150.0).show_ui(ui, |ui| {
            for t in ed.tracks.clone() {
                if ui.selectable_label(t == ed.track, &t).clicked() {
                    pick = Some(t);
                }
            }
        });
        if let Some(t) = pick {
            ed.open(&t);
        }
        let save = if ed.dirty() { "Save ●" } else { "Save" };
        if ui.button(save).on_hover_text("Ctrl+S: write music/<track>.score").clicked() {
            ed.save();
        }
        ui.menu_button("Save as…", |ui| {
            ui.label("New track name");
            let r = ui.add(egui::TextEdit::singleline(&mut ed.new_name).desired_width(160.0).hint_text("hall_night"));
            if ui.button("Save copy").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                let n = std::mem::take(&mut ed.new_name);
                ed.save_as(&n);
                ui.close();
            }
        });
        ui.add_space(16.0);
        let playing = ed.status.playing;
        if ui.add(egui::Button::new(if playing { "■ Stop" } else { "▶ Play" }).min_size(vec2(76.0, 0.0))).on_hover_text("Space").clicked() {
            ed.toggle_play();
        }
        if ui.button("▶ Play a round").on_hover_text("R: he hears you, loses you, spots you, fights, and you win").clicked() {
            ed.play_round();
        }
        ui.add_space(16.0);
        for (v, name) in [(View::Score, "Score"), (View::Patches, "Patches"), (View::Text, "Text")] {
            if ui.selectable_label(ed.view == v, name).clicked() {
                ed.view = v;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("audio: {}", ed.out.device)).color(FAINT).size(11.0));
            let s = &ed.status;
            if s.playing {
                let mut t = format!("{} · bar {} · beat {} · {:.0} bpm", s.cue, s.bar + 1, (s.step % STEPS_PER_BAR) / 4 + 1, s.bpm);
                if let Some(p) = &s.pending {
                    t += &format!("   → {p} {}", if p == "win" || p == "lose" { "on the next beat" } else { "on the bar line" });
                }
                ui.label(RichText::new(t).color(cue_color(&s.cue)).monospace());
            } else {
                ui.label(RichText::new("stopped").color(MUTED).monospace());
            }
        });
    });
}

/// HIM: the simulated game on top, the mixer below.
fn him(ed: &mut Editor, ui: &mut egui::Ui) {
    ui.label(RichText::new("HIS STATE").size(11.0).color(MUTED));
    let color = match ed.ended {
        Some(true) => CALM,
        Some(false) => HOT,
        None if ed.engaged => HOT,
        None if ed.suspicion > 0.3 => WARN,
        None => CALM,
    };
    ui.label(RichText::new(ed.caption()).size(28.0).strong().color(color));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("suspicion").color(MUTED));
        ui.spacing_mut().slider_width = 150.0;
        if ui.add(egui::Slider::new(&mut ed.suspicion, 0.0..=1.0).fixed_decimals(2)).on_hover_text("[ and ]").changed() {
            ed.round = None;
        }
    });
    ui.horizontal(|ui| {
        let mut on = ed.engaged;
        if ui.toggle_value(&mut on, "He engages").on_hover_text("E").changed() {
            ed.set_engaged(on);
        }
        if ui.button("You win").on_hover_text("W").clicked() {
            ed.end(true);
        }
        if ui.button("You die").on_hover_text("L").clicked() {
            ed.end(false);
        }
    });
    if let Some(start) = ed.round {
        ui.label(RichText::new(format!("scripted round · {:.0}s", ed.now - start)).color(MUTED).size(11.0));
    }
    ui.add_space(8.0);
    ui.separator();
    ui.label(RichText::new("MIXER").size(11.0).color(MUTED));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| mixer(ed, ui));
}

fn mixer(ed: &mut Editor, ui: &mut egui::Ui) {
    let stems = ed.status.stems.clone();
    for cue in ed.doc.cues.clone() {
        let active = ed.status.playing && ed.status.cue == cue.name;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let c = cue_color(&cue.name);
            let text = RichText::new(cue.name.to_uppercase()).size(12.0).color(if active { c } else { MUTED });
            let text = if active { text.strong().underline() } else { text };
            ui.label(text);
            ui.label(RichText::new(format!("{:.0} bpm", cue.bpm)).size(11.0).color(FAINT));
        });
        for st in stems.iter().filter(|s| s.cue == cue.name) {
            ui.horizontal(|ui| {
                let (muted, soloed) = {
                    let e = ed.out.engine.lock().unwrap();
                    (e.muted.contains(&st.name), e.soloed.contains(&st.name))
                };
                let small = |t: &str, on: bool, col: Color32| egui::Button::new(RichText::new(t).size(10.0).color(if on { BG } else { MUTED })).fill(if on { col } else { RAISED }).min_size(vec2(18.0, 16.0));
                if ui.add(small("M", muted, WARN)).on_hover_text("mute").clicked() {
                    let mut e = ed.out.engine.lock().unwrap();
                    if !e.muted.remove(&st.name) {
                        e.muted.insert(st.name.clone());
                    }
                }
                if ui.add(small("S", soloed, CALM)).on_hover_text("solo").clicked() {
                    let mut e = ed.out.engine.lock().unwrap();
                    if !e.soloed.remove(&st.name) {
                        e.soloed.insert(st.name.clone());
                    }
                }
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
                let p = ui.painter();
                p.rect_filled(rect, 2.0, RAISED);
                let fader = rect.with_max_x(rect.min.x + rect.width() * st.gain.clamp(0.0, 1.5) / 1.5);
                p.rect_filled(fader, 2.0, cue_color(&cue.name).gamma_multiply(0.35));
                let peak = (st.meter * 4.0).clamp(0.0, 1.0);
                let m = egui::Rect::from_min_max(pos2(rect.min.x, rect.max.y - 3.0), pos2(rect.min.x + rect.width() * peak, rect.max.y));
                p.rect_filled(m, 0.0, FG);
                p.text(rect.left_center() + vec2(6.0, -1.0), Align2::LEFT_CENTER, &st.name, FontId::proportional(11.0), FG);
            });
        }
    }
}

// ── Score view ───────────────────────────────────────────────────────────────────

const HEAD_W: f32 = 236.0;
const CELL_W: f32 = 30.0;
const ROW_H: f32 = 52.0;

fn role_note(role: Role) -> &'static str {
    match role {
        Role::Calm => "Loops while he isn't engaged. His suspicion moves each stem's fader along its rule.",
        Role::Fight => "Loops while he's engaged. Switches in on the bar line, from the top, at its own tempo.",
        Role::Win => "Plays once, from the next beat, when you kill him.",
        Role::Lose => "Plays once, from the next beat, when he kills you.",
        Role::Other => "Not used by the game yet. Plays when you press ▶ here.",
    }
}

fn score_view(ed: &mut Editor, ui: &mut egui::Ui) {
    if ed.doc.cues.is_empty() {
        return;
    }
    ed.cue = ed.cue.min(ed.doc.cues.len() - 1);
    let mut changed = false;
    // Cue tabs.
    ui.horizontal(|ui| {
        for (i, c) in ed.doc.cues.iter().enumerate() {
            let col = cue_color(&c.name);
            let text = RichText::new(c.name.to_uppercase()).strong().color(if i == ed.cue { BG } else { col });
            if ui.add(egui::Button::new(text).fill(if i == ed.cue { col } else { RAISED })).clicked() {
                ed.cue = i;
                ed.sel = None;
            }
        }
        ui.menu_button("+ cue", |ui| {
            for name in ["fight", "win", "lose", "search"] {
                if !ed.doc.cues.iter().any(|c| c.name == name) && ui.button(name).clicked() {
                    let bpm = ed.doc.cues[0].bpm;
                    ed.doc.cues.push(Cue { name: name.into(), bpm, chords: ed.doc.cues[0].chords.clone(), fade: 0.7 });
                    ed.cue = ed.doc.cues.len() - 1;
                    changed = true;
                    ui.close();
                }
            }
        });
    });
    let ci = ed.cue;
    let role = Role::of(&ed.doc.cues[ci].name);
    ui.horizontal(|ui| {
        let cue = &mut ed.doc.cues[ci];
        ui.label(RichText::new("bpm").color(MUTED));
        changed |= ui.add(egui::DragValue::new(&mut cue.bpm).range(20.0..=300.0).speed(0.5)).changed();
        ui.label(RichText::new("chords").color(MUTED));
        let ok = theory::chord_line(&cue.chords).is_ok();
        let r = ui.add(egui::TextEdit::singleline(&mut cue.chords).desired_width(240.0).text_color(if ok { FG } else { HOT }));
        if r.changed() && theory::chord_line(&cue.chords).is_ok() {
            changed = true;
        }
        r.on_hover_text("One chord per bar: Dm, Bb, A7b9, Ebmaj7, Bm7b5, Dsus4… `%` repeats the bar before.");
        ui.label(RichText::new("fade").color(MUTED));
        changed |= ui.add(egui::DragValue::new(&mut cue.fade).range(0.005..=5.0).speed(0.01).suffix(" s")).on_hover_text("How fast the faders move in this cue. Small: the layers hit. Large: they swell.").changed();
        let name = cue.name.clone();
        if ui.button(format!("▶ {name}")).on_hover_text("Play from the top of this cue").clicked() {
            ed.play_cue(ci);
        }
        if role != Role::Calm && ui.button("Delete cue").clicked() {
            ed.doc.stems.retain(|s| s.cue != name);
            ed.doc.cues.remove(ci);
            ed.cue = 0;
            changed = true;
        }
    });
    ui.label(RichText::new(role_note(role)).color(MUTED).size(12.0));
    ui.add_space(4.0);
    if changed {
        ed.commit();
        return;
    }
    cell_editor(ed, ui);
    ui.add_space(4.0);
    grid(ed, ui);
}

/// The selected cell's token, editable.
fn cell_editor(ed: &mut Editor, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let Some((si, step)) = ed.sel.filter(|(si, _)| *si < ed.doc.stems.len()) else {
            ui.label(RichText::new("Click a cell to edit it. Arrows move, Delete rests, _ holds, Enter types.").color(FAINT).size(12.0));
            return;
        };
        ui.label(RichText::new(format!("{} · bar {} · step {}", ed.doc.stems[si].name, step / STEPS_PER_BAR + 1, step % STEPS_PER_BAR + 1)).color(MUTED));
        let id = egui::Id::new("cell");
        let r = ui.add(egui::TextEdit::singleline(&mut ed.cell).id(id).desired_width(220.0).font(egui::TextStyle::Monospace).text_color(if ed.cell_error.is_some() { HOT } else { FG }));
        if r.changed() {
            ed.cell_error = pattern::step(ed.cell.trim()).err();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            if set_cell(ed, si, step, ed.cell.trim().to_string()) {
                let next = (step + 1) % part_len(ed, si);
                select(ed, si, next);
                ui.memory_mut(|m| m.request_focus(id));
            }
        }
        if let Some(e) = &ed.cell_error {
            ui.label(RichText::new(e).color(HOT).size(12.0));
        }
        ui.label(RichText::new("? syntax").color(MUTED)).on_hover_text(
            ". rest   _ hold   x hit (X accented)\n\
             r  r-12  r+7   chord root ± semitones\n\
             1 2 3 4  3+12  chord tones: root, 3rd, 5th, 7th\n\
             ch  ch+12      the whole chord, voiced open\n\
             d4  bb2  f#5   absolute notes\n\
             r-12,r,3       stack with commas\n\
             suffix ! accent, ? ghost",
        );
    });
}

fn part_tokens(ed: &Editor, si: usize) -> Vec<String> {
    pattern::tokens(&ed.doc.stems[si].part).unwrap_or_default()
}

fn part_len(ed: &Editor, si: usize) -> usize {
    part_tokens(ed, si).len().max(1)
}

fn select(ed: &mut Editor, si: usize, step: usize) {
    ed.sel = Some((si, step));
    ed.cell = part_tokens(ed, si).get(step).cloned().unwrap_or_else(|| ".".into());
    ed.cell_error = None;
}

fn set_cell(ed: &mut Editor, si: usize, step: usize, tok: String) -> bool {
    let tok = if tok.is_empty() { ".".to_string() } else { tok };
    if let Err(e) = pattern::step(&tok) {
        ed.cell_error = Some(e);
        return false;
    }
    let mut t = part_tokens(ed, si);
    if step >= t.len() {
        return false;
    }
    t[step] = tok;
    ed.doc.stems[si].part = pattern::format(&t);
    ed.cell_error = None;
    ed.commit();
    true
}

fn grid_keys(ed: &mut Editor, ctx: &egui::Context) {
    let Some((si, step)) = ed.sel else { return };
    if si >= ed.doc.stems.len() {
        return;
    }
    let len = part_len(ed, si);
    let (l, r, del, hold, enter) = ctx.input(|i| {
        (
            i.key_pressed(Key::ArrowLeft),
            i.key_pressed(Key::ArrowRight),
            i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace) || i.key_pressed(Key::Period),
            i.key_pressed(Key::Minus) && i.modifiers.shift,
            i.key_pressed(Key::Enter),
        )
    });
    let (up, down) = ctx.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown)));
    if l {
        select(ed, si, (step + len - 1) % len);
    }
    if r {
        select(ed, si, (step + 1) % len);
    }
    if up || down {
        let cue = ed.doc.cues[ed.cue].name.clone();
        let rows: Vec<usize> = (0..ed.doc.stems.len()).filter(|&i| ed.doc.stems[i].cue == cue).collect();
        if let Some(k) = rows.iter().position(|&i| i == si) {
            let k = if up { k.saturating_sub(1) } else { (k + 1).min(rows.len() - 1) };
            let s = step.min(part_len(ed, rows[k]) - 1);
            select(ed, rows[k], s);
        }
    }
    if del && set_cell(ed, si, step, ".".into()) {
        select(ed, si, (step + 1) % len);
    }
    if hold && set_cell(ed, si, step, "_".into()) {
        select(ed, si, (step + 1) % len);
    }
    if enter {
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("cell")));
    }
}

fn grid(ed: &mut Editor, ui: &mut egui::Ui) {
    let ci = ed.cue;
    let cue_name = ed.doc.cues[ci].name.clone();
    let chords: Vec<String> = ed.doc.cues[ci].chords.split_whitespace().filter(|t| *t != "|").map(String::from).collect();
    let bars = chords.len().max(1);
    let rows: Vec<usize> = (0..ed.doc.stems.len()).filter(|&i| ed.doc.stems[i].cue == cue_name).collect();
    let col = cue_color(&cue_name);
    let lens: Vec<usize> = rows.iter().map(|&i| part_len(ed, i)).collect();
    let cols = lens.iter().copied().chain([bars * STEPS_PER_BAR]).max().unwrap_or(16);
    let playing_here = ed.status.playing && ed.status.cue == cue_name;
    let now = ed.status.step;
    let mut action: Option<Action> = None;

    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        // Chord lane.
        let (lane, _) = ui.allocate_exact_size(vec2(HEAD_W + cols as f32 * CELL_W, 24.0), Sense::hover());
        let p = ui.painter();
        p.text(lane.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, "chords", FontId::proportional(11.0), MUTED);
        for b in 0..cols.div_ceil(STEPS_PER_BAR) {
            let x = lane.min.x + HEAD_W + (b * STEPS_PER_BAR) as f32 * CELL_W;
            let ghost = b >= bars;
            let name = &chords[b % chords.len().max(1)];
            p.text(pos2(x + 6.0, lane.center().y), Align2::LEFT_CENTER, format!("{}  {}", b + 1, name), FontId::monospace(13.0), if ghost { FAINT } else { col });
        }
        for (k, &si) in rows.iter().enumerate() {
            let tokens = part_tokens(ed, si);
            let len = lens[k];
            let (row, _) = ui.allocate_exact_size(vec2(HEAD_W + cols as f32 * CELL_W, ROW_H), Sense::hover());
            // Header.
            let head = egui::Rect::from_min_size(row.min, vec2(HEAD_W - 8.0, ROW_H));
            ui.painter().rect_filled(head, 3.0, PANEL);
            let mut hu = ui.new_child(egui::UiBuilder::new().max_rect(head.shrink(4.0)).layout(egui::Layout::top_down(egui::Align::Min)));
            hu.spacing_mut().item_spacing = vec2(4.0, 3.0);
            if let Some(a) = stem_header(ed, &mut hu, si, role_of(ed, ci)) {
                action = Some(a);
            }
            // Cells.
            let p = ui.painter();
            for c in 0..cols {
                let rect = egui::Rect::from_min_size(pos2(row.min.x + HEAD_W + c as f32 * CELL_W, row.min.y + 4.0), vec2(CELL_W, ROW_H - 8.0));
                let ghost = c >= len;
                let i = c % len;
                let tok = tokens.get(i).map(String::as_str).unwrap_or(".");
                let beat_bg = if (c / 4) % 2 == 0 { RAISED } else { Color32::from_rgb(0x16, 0x21, 0x24) };
                p.rect_filled(rect.shrink2(vec2(0.5, 0.0)), 0.0, beat_bg);
                match tok {
                    "." => {}
                    "_" => {
                        let strip = egui::Rect::from_center_size(rect.center(), vec2(CELL_W, 8.0));
                        p.rect_filled(strip, 0.0, col.gamma_multiply(if ghost { 0.12 } else { 0.35 }));
                    }
                    _ => {
                        let accent = tok.ends_with('!') || tok == "X";
                        let ghostnote = tok.ends_with('?');
                        let a = if ghost { 0.15 } else if accent { 0.95 } else if ghostnote { 0.4 } else { 0.7 };
                        p.rect_filled(rect.shrink2(vec2(1.5, 6.0)), 3.0, col.gamma_multiply(a));
                        // The first pitch; a stack shows how many it holds.
                        let body = tok.trim_end_matches(['!', '?']);
                        let parts: Vec<&str> = body.split(',').collect();
                        let first: String = parts[0].chars().take(5).collect();
                        let ink = if ghost { FAINT } else { BG };
                        if parts.len() > 1 {
                            p.text(rect.center() - vec2(0.0, 4.0), Align2::CENTER_CENTER, first, FontId::monospace(9.0), ink);
                            p.text(rect.center() + vec2(0.0, 7.0), Align2::CENTER_CENTER, format!("+{}", parts.len() - 1), FontId::monospace(8.0), ink);
                        } else {
                            p.text(rect.center(), Align2::CENTER_CENTER, first, FontId::monospace(9.0), ink);
                        }
                    }
                }
                if c % STEPS_PER_BAR == 0 {
                    p.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(1.5, BAR_LINE));
                }
                if playing_here && i == now % len && !ghost {
                    p.rect_stroke(rect, 0.0, Stroke::new(2.0, FG), StrokeKind::Inside);
                }
                if ed.sel == Some((si, i)) && !ghost {
                    p.rect_stroke(rect, 2.0, Stroke::new(2.0, WARN), StrokeKind::Inside);
                }
                let resp = ui.interact(rect, egui::Id::new(("cell", si, c)), Sense::click());
                let resp = if tok.len() > 4 { resp.on_hover_text(tok) } else { resp };
                if resp.clicked() {
                    action = Some(Action::Select(si, i));
                }
            }
            ui.add_space(2.0);
        }
        ui.add_space(8.0);
        if ui.button("+ stem").clicked() {
            action = Some(Action::Add);
        }
    });

    match action {
        Some(Action::Select(si, i)) => select(ed, si, i),
        Some(Action::Add) => {
            let patch = ed.doc.patches.keys().next().cloned().unwrap_or_default();
            let n = ed.doc.stems.len() + 1;
            ed.doc.stems.push(Stem { name: format!("Stem {n}"), cue: cue_name, patch, level: 1.0, fader: Fader::Always, once: false, part: "r .*15".into() });
            ed.commit();
        }
        Some(Action::Changed) => ed.commit(),
        Some(Action::Delete(si)) => {
            ed.doc.stems.remove(si);
            ed.sel = None;
            ed.commit();
        }
        Some(Action::Duplicate(si)) => {
            let mut s = ed.doc.stems[si].clone();
            s.name += " 2";
            ed.doc.stems.insert(si + 1, s);
            ed.commit();
        }
        Some(Action::Move(si, up)) => {
            let cue = ed.doc.stems[si].cue.clone();
            let same: Vec<usize> = (0..ed.doc.stems.len()).filter(|&i| ed.doc.stems[i].cue == cue).collect();
            let k = same.iter().position(|&i| i == si).unwrap_or(0);
            let other = if up { k.checked_sub(1).map(|k| same[k]) } else { same.get(k + 1).copied() };
            if let Some(o) = other {
                ed.doc.stems.swap(si, o);
                ed.sel = None;
                ed.commit();
            }
        }
        Some(Action::Resize(si, delta)) => {
            let mut t = part_tokens(ed, si);
            if delta > 0 {
                t.extend(std::iter::repeat_n(".".to_string(), STEPS_PER_BAR));
            } else if t.len() > STEPS_PER_BAR {
                t.truncate(t.len() - STEPS_PER_BAR);
            }
            ed.doc.stems[si].part = pattern::format(&t);
            ed.sel = None;
            ed.commit();
        }
        Some(Action::Double(si)) => {
            let t = part_tokens(ed, si);
            let t: Vec<String> = t.iter().chain(t.iter()).cloned().collect();
            ed.doc.stems[si].part = pattern::format(&t);
            ed.commit();
        }
        None => {}
    }
}

fn role_of(ed: &Editor, ci: usize) -> Role {
    Role::of(&ed.doc.cues[ci].name)
}

enum Action {
    Select(usize, usize),
    Add,
    Changed,
    Delete(usize),
    Duplicate(usize),
    Move(usize, bool),
    Resize(usize, i32),
    Double(usize),
}

fn stem_header(ed: &mut Editor, ui: &mut egui::Ui, si: usize, role: Role) -> Option<Action> {
    let mut action = None;
    let patches: Vec<String> = ed.doc.patches.keys().cloned().collect();
    let len = part_len(ed, si);
    let stem = &mut ed.doc.stems[si];
    ui.horizontal(|ui| {
        if ui.add(egui::TextEdit::singleline(&mut stem.name).desired_width(118.0).font(egui::TextStyle::Body)).lost_focus() {
            action = Some(Action::Changed);
        }
        egui::ComboBox::from_id_salt(("patch", si)).selected_text(RichText::new(&stem.patch).size(12.0)).width(78.0).show_ui(ui, |ui| {
            for p in &patches {
                if ui.selectable_value(&mut stem.patch, p.clone(), p).clicked() {
                    action = Some(Action::Changed);
                }
            }
        });
        ui.menu_button("•••", |ui| {
            if ui.checkbox(&mut stem.once, "Only the first time through").on_hover_text("An entrance hit: plays on the cue's first pass, then sits out").changed() {
                action = Some(Action::Changed);
            }
            ui.label(RichText::new(format!("Part: {} bars ({len} steps)", len.div_ceil(STEPS_PER_BAR))).color(MUTED));
            ui.horizontal(|ui| {
                if ui.button("+ bar").clicked() {
                    action = Some(Action::Resize(si, 1));
                }
                if ui.button("− bar").clicked() {
                    action = Some(Action::Resize(si, -1));
                }
                if ui.button("Double").on_hover_text("Repeat the part once more, to vary the copy").clicked() {
                    action = Some(Action::Double(si));
                }
            });
            ui.separator();
            if ui.button("Move up").clicked() {
                action = Some(Action::Move(si, true));
            }
            if ui.button("Move down").clicked() {
                action = Some(Action::Move(si, false));
            }
            if ui.button("Duplicate").clicked() {
                action = Some(Action::Duplicate(si));
            }
            if ui.button(RichText::new("Delete").color(HOT)).clicked() {
                action = Some(Action::Delete(si));
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("lvl").size(11.0).color(MUTED));
        if ui.add(egui::DragValue::new(&mut stem.level).range(0.0..=2.0).speed(0.01).max_decimals(2)).changed() {
            action = Some(Action::Changed);
        }
        if role == Role::Calm {
            let kind = match stem.fader {
                Fader::Always => 0,
                Fader::Rise { .. } => 1,
                Fader::Fall { .. } => 2,
            };
            let mut k = kind;
            egui::ComboBox::from_id_salt(("fader", si)).selected_text(["always", "rises", "falls"][k]).width(56.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut k, 0, "always").on_hover_text("Up whatever his suspicion");
                ui.selectable_value(&mut k, 1, "rises").on_hover_text("Fades in as his suspicion rises");
                ui.selectable_value(&mut k, 2, "falls").on_hover_text("Fades out as his suspicion rises");
            });
            let (mut from, mut to) = match stem.fader {
                Fader::Rise { from, to } | Fader::Fall { from, to } => (from, to),
                Fader::Always => (0.3, 0.6),
            };
            if k != 0 {
                let a = ui.add(egui::DragValue::new(&mut from).range(0.0..=1.0).speed(0.01).max_decimals(2)).on_hover_text("suspicion where the fade starts");
                let b = ui.add(egui::DragValue::new(&mut to).range(0.0..=1.0).speed(0.01).max_decimals(2)).on_hover_text("suspicion where it's done");
                if a.changed() || b.changed() {
                    action = Some(Action::Changed);
                }
            }
            let new = match k {
                1 => Fader::Rise { from, to },
                2 => Fader::Fall { from, to },
                _ => Fader::Always,
            };
            if new != stem.fader {
                stem.fader = new;
                action = Some(Action::Changed);
            }
        }
        if stem.once {
            ui.label(RichText::new("once").size(11.0).color(WARN));
        }
    });
    action
}

// ── Patches view ─────────────────────────────────────────────────────────────────

fn patches_view(ed: &mut Editor, ui: &mut egui::Ui) {
    let names: Vec<String> = ed.doc.patches.keys().cloned().collect();
    if ed.patch.as_ref().is_none_or(|p| !ed.doc.patches.contains_key(p)) {
        ed.patch = names.first().cloned();
    }
    let mut changed = false;
    egui::Panel::left("patch_list").exact_size(190.0).resizable(false).frame(egui::Frame::new().inner_margin(egui::Margin { right: 12, ..Default::default() })).show_inside(ui, |ui| {
        ui.label(RichText::new("PATCHES").size(11.0).color(MUTED));
        egui::ScrollArea::vertical().max_height(ui.available_height() - 70.0).show(ui, |ui| {
            for n in &names {
                let used = ed.doc.stems.iter().filter(|s| &s.patch == n).count();
                let r = ui.selectable_label(ed.patch.as_ref() == Some(n), format!("{n}  ·{used}"));
                if r.clicked() {
                    ed.patch = Some(n.clone());
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("+ new").clicked() {
                let name = unique(&ed.doc.patches, "patch");
                ed.doc.patches.insert(name.clone(), Patch::default());
                ed.patch = Some(name);
                changed = true;
            }
            if let Some(cur) = ed.patch.clone()
                && ui.button("Duplicate").clicked()
            {
                let name = unique(&ed.doc.patches, &cur);
                let p = ed.doc.patches[&cur].clone();
                ed.doc.patches.insert(name.clone(), p);
                ed.patch = Some(name);
                changed = true;
            }
        });
    });
    let Some(name) = ed.patch.clone() else {
        if changed {
            ed.commit();
        }
        return;
    };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&name).size(22.0).strong());
            let used: Vec<String> = ed.doc.stems.iter().filter(|s| s.patch == name).map(|s| s.name.clone()).collect();
            ui.label(RichText::new(if used.is_empty() { "unused".to_string() } else { format!("used by {}", used.join(", ")) }).color(MUTED));
            if used.is_empty() && ui.button(RichText::new("Delete").color(HOT)).clicked() {
                ed.doc.patches.remove(&name);
                ed.patch = None;
                changed = true;
            }
        });
        let mut rename = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Hear it").color(MUTED));
            for (label, notes, gate) in [
                ("low", vec![38], 0.6),
                ("mid", vec![50], 0.6),
                ("high", vec![74], 0.6),
                ("Dm chord", vec![50, 57, 62, 65], 1.2),
                ("long", vec![57], 3.0),
            ] {
                if ui.button(label).clicked() {
                    ed.audition_patch(&name, &notes, gate);
                }
            }
            ui.add_space(20.0);
            ui.menu_button("Rename…", |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut ed.new_name).desired_width(140.0));
                if ui.button("Rename").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                    rename = Some(std::mem::take(&mut ed.new_name));
                    ui.close();
                }
            });
        });
        if let Some(new) = rename {
            let new = new.trim().replace(' ', "_");
            if !new.is_empty() && !ed.doc.patches.contains_key(&new) {
                let p = ed.doc.patches.remove(&name).expect("selected patch exists");
                ed.doc.patches.insert(new.clone(), p);
                for s in &mut ed.doc.stems {
                    if s.patch == name {
                        s.patch = new.clone();
                    }
                }
                ed.patch = Some(new);
                ed.commit();
                return;
            }
        }
        let Some(p) = ed.doc.patches.get_mut(&name) else { return };
        changed |= ui.add(egui::TextEdit::singleline(&mut p.about).desired_width(560.0).hint_text("what it should sound like")).lost_focus();
        ui.add_space(6.0);
        changed |= patch_editor(ui, p);
    });
    if changed {
        ed.commit();
    }
}

fn unique(map: &std::collections::BTreeMap<String, Patch>, base: &str) -> String {
    (2..).map(|n| format!("{base}_{n}")).find(|n| !map.contains_key(n)).expect("some name is free")
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).size(11.0).color(MUTED));
}

fn slider(ui: &mut egui::Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, log: bool, hint: &str) -> bool {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(96.0, 18.0), egui::Label::new(RichText::new(label).color(FG)));
        ui.spacing_mut().slider_width = 260.0;
        ui.add(egui::Slider::new(v, range).logarithmic(log).max_decimals(3)).on_hover_text(hint).changed()
    })
    .inner
}

fn patch_editor(ui: &mut egui::Ui, p: &mut Patch) -> bool {
    let mut ch = false;
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        section(ui, "OSCILLATORS");
        let mut remove = None;
        for (i, o) in p.osc.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("wave", i)).selected_text(o.wave.name()).width(80.0).show_ui(ui, |ui| {
                    for w in Wave::ALL {
                        ch |= ui.selectable_value(&mut o.wave, w, w.name()).changed();
                    }
                });
                ch |= ui.add(egui::DragValue::new(&mut o.semis).range(-36.0..=36.0).speed(0.1).prefix("semi ")).changed();
                ch |= ui.add(egui::DragValue::new(&mut o.cents).range(-100.0..=100.0).speed(0.2).prefix("ct ")).changed();
                ch |= ui.add(egui::DragValue::new(&mut o.ratio).range(0.1..=16.0).speed(0.005).prefix("× ")).changed();
                ch |= ui.add(egui::DragValue::new(&mut o.level).range(0.0..=2.0).speed(0.01).prefix("lvl ")).changed();
                if ui.small_button("x").on_hover_text("remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove.filter(|_| p.osc.len() > 1) {
            p.osc.remove(i);
            ch = true;
        }
        if ui.button("+ oscillator").clicked() {
            p.osc.push(Osc { cents: 7.0, ..Osc::default() });
            ch = true;
        }
        section(ui, "ENVELOPE");
        ch |= slider(ui, "attack", &mut p.attack, 0.001..=8.0, true, "seconds to full");
        ch |= slider(ui, "decay", &mut p.decay, 0.001..=4.0, true, "seconds down to the sustain level");
        ch |= slider(ui, "sustain", &mut p.sustain, 0.0..=1.0, false, "level held while the note is; 0 = a one-shot that rings out its decay");
        ch |= slider(ui, "release", &mut p.release, 0.005..=4.0, true, "seconds to silence after the note");
        envelope_plot(ui, p);
        let ui = &mut cols[1];
        section(ui, "FILTER");
        ui.horizontal(|ui| {
            for (k, n) in [(FilterKind::None, "off"), (FilterKind::Lowpass, "lowpass"), (FilterKind::Highpass, "highpass"), (FilterKind::Bandpass, "bandpass")] {
                ch |= ui.selectable_value(&mut p.filter.kind, k, n).changed();
            }
        });
        if p.filter.kind != FilterKind::None {
            ch |= slider(ui, "cutoff", &mut p.filter.cutoff, 20.0..=18000.0, true, "Hz");
            ch |= slider(ui, "resonance", &mut p.filter.q, 0.1..=10.0, true, "Q");
            ch |= slider(ui, "env amount", &mut p.filter.env, -4000.0..=8000.0, false, "Hz the cutoff opens by at each note (accents open it more)");
            ch |= slider(ui, "env attack", &mut p.filter.env_attack, 0.0..=2.0, false, "seconds the filter takes to open: brass swells");
            ch |= slider(ui, "env decay", &mut p.filter.env_decay, 0.005..=4.0, true, "seconds it takes to close again");
        }
        section(ui, "CHARACTER");
        ch |= slider(ui, "gain", &mut p.gain, 0.001..=2.0, true, "level of each note");
        ch |= slider(ui, "drive", &mut p.drive, 0.0..=8.0, false, "saturation before the filter: grit for low brass");
        ch |= slider(ui, "tremolo Hz", &mut p.tremolo_rate, 0.0..=30.0, false, "bowed tremolo speed");
        ch |= slider(ui, "tremolo", &mut p.tremolo_depth, 0.0..=1.0, false, "tremolo depth");
        ch |= slider(ui, "glide", &mut p.glide, -24.0..=24.0, false, "semitones the pitch slides over a held note: risers");
        ch |= slider(ui, "thump", &mut p.drop, 0.0..=4.0, false, "drum pitch drop: starts this much higher (×) and falls to the note");
        ch |= slider(ui, "pan", &mut p.pan, -1.0..=1.0, false, "left/right");
        ch |= slider(ui, "reverb send", &mut p.send, 0.0..=2.0, false, "how much goes to the hall");
        ui.horizontal(|ui| {
            ui.add_sized(vec2(96.0, 18.0), egui::Label::new("hit pitch"));
            let mut hit = p.hit.clone().unwrap_or_default();
            let ok = hit.is_empty() || pattern::step(&hit).is_ok();
            if ui.add(egui::TextEdit::singleline(&mut hit).desired_width(80.0).hint_text("r").text_color(if ok { FG } else { HOT })).on_hover_text("What `x` plays: r-24 (two octaves under the chord root), d2, …").changed() {
                let ok = hit.is_empty() || pattern::step(&hit).is_ok_and(|s| matches!(s, pattern::Step::Notes(..)));
                if ok {
                    p.hit = (!hit.is_empty()).then_some(hit);
                    ch = true;
                }
            }
        });
    });
    ch
}

fn envelope_plot(ui: &mut egui::Ui, p: &Patch) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(420.0), 70.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, RAISED);
    let gate = if p.sustain <= 0.0 { 0.2 } else { (p.attack + p.decay + 0.4).min(4.0) };
    let total = second_person::music::voice::length(p, gate as f64) as f32;
    let pts: Vec<_> = (0..=120)
        .map(|i| {
            let t = total * i as f32 / 120.0;
            let y = second_person::music::voice::amp(p, gate, t);
            pos2(rect.min.x + rect.width() * i as f32 / 120.0, rect.max.y - 4.0 - (rect.height() - 8.0) * y.clamp(0.0, 1.0))
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, CALM)));
    if p.sustain > 0.0 {
        let x = rect.min.x + rect.width() * gate / total;
        painter.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], Stroke::new(1.0, FAINT));
        painter.text(pos2(x + 4.0, rect.min.y + 8.0), Align2::LEFT_CENTER, "note off", FontId::proportional(10.0), FAINT);
    }
    painter.text(rect.right_bottom() - vec2(4.0, 4.0), Align2::RIGHT_BOTTOM, format!("{total:.2}s"), FontId::proportional(10.0), MUTED);
}

// ── Text view ────────────────────────────────────────────────────────────────────

fn text_view(ed: &mut Editor, ui: &mut egui::Ui) {
    ui.label(RichText::new("The track as saved. Edits apply as soon as they parse; saving from here keeps your comments and layout.").color(MUTED).size(12.0));
    if let Some(e) = &ed.text_error {
        ui.label(RichText::new(e).color(HOT).monospace().size(12.0));
    }
    let mut text = ed.text.clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let r = ui.add_sized(
            Vec2::new(ui.available_width(), ui.available_height().max(400.0)),
            egui::TextEdit::multiline(&mut text).code_editor().desired_rows(40).lock_focus(true),
        );
        if r.changed() {
            ed.set_text(text);
        }
    });
}
