//! HUD drawn with egui on a dedicated overlay camera.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, PrimaryEguiContext, egui};
use rand::seq::IndexedRandom;

use crate::{
    arena::Layout,
    combat::{ShooterHit, TargetHit},
    layout::ScreenLayout,
    radar::RadarMode,
    round::{GameState, TargetMobile},
    shooter::{SHOOTER_MAX_HP, Shooter, Stunned},
    start::Started,
    target::{Activity, Suspicion, TARGET_MAX_HP, Target},
    touch::{TouchControls, TouchWhistle, WHISTLE_SIZE},
};

/// Screen flashes: red when you're hit; a hit marker and a light flash when the target is hit.
/// `ended` counts seconds since the round ended (0 while playing).
#[derive(Resource, Default)]
struct Flashes {
    hurt: f32,
    hit: f32,
    ended: f32,
    /// End-of-round headline, picked when the round ends.
    banner: &'static str,
}

/// Hold the end banner back so you can watch him (or yourself) go down.
const BANNER_DELAY: f32 = 1.2;

/// You died. One is picked at random each time.
const DEATH_BANNERS: &[&str] = &[
    "Oh dear, you are dead!",
    "YOU DIED",
    "WASTED",
    "Snake? Snake?! SNAAAKE!",
    "Game over, man! Game over!",
    "You have died of dysentery.",
    "You tried to ford the river.",
    "Mission failed. We'll get 'em next time.",
    "Turns out he shoots back.",
    "The hunter became the hunted.",
];

/// He died.
const WIN_BANNERS: &[&str] = &[
    "Congratulations, you just advanced a Slayer level.",
    "ENEMY FELLED",
    "Target eliminated. Agent 47 would be proud.",
    "He's not coming back.",
    "Mission accomplished.",
    "You have slain the target.",
    "And stay down.",
    "YEEEEEEHAWWW!",
];

/// Won without taking a scratch.
const FLAWLESS_BANNER: &str = "FLAWLESS VICTORY";

pub fn plugin(app: &mut App) {
    app.init_resource::<Flashes>()
        .add_systems(Startup, spawn_hud_camera)
        .add_systems(Update, track_flashes)
        .add_systems(EguiPrimaryContextPass, draw_hud);
}

fn spawn_hud_camera(mut commands: Commands) {
    commands.spawn((
        Name::new("HUD camera"),
        Camera2d,
        Camera {
            order: 10,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        PrimaryEguiContext,
        // bevy_ui (the touch stick) draws here too, above every game viewport.
        IsDefaultUiCamera,
    ));
}

fn track_flashes(
    time: Res<Time>,
    state: Res<State<GameState>>,
    mut flashes: ResMut<Flashes>,
    mut hurt: MessageReader<ShooterHit>,
    mut hit: MessageReader<TargetHit>,
    shooter: Option<Single<&Shooter>>,
) {
    let decay = time.delta_secs() * 2.5;
    if flashes.ended == 0.0 && *state.get() != GameState::Playing {
        let flawless = shooter.is_some_and(|s| s.hp >= SHOOTER_MAX_HP);
        flashes.banner = match state.get() {
            GameState::Won if flawless => FLAWLESS_BANNER,
            GameState::Won => WIN_BANNERS.choose(&mut rand::rng()).unwrap(),
            _ => DEATH_BANNERS.choose(&mut rand::rng()).unwrap(),
        };
    }
    flashes.hurt = (flashes.hurt - decay).max(0.0);
    flashes.hit = (flashes.hit - decay).max(0.0);
    flashes.ended = match state.get() {
        GameState::Playing => 0.0,
        _ => flashes.ended + time.delta_secs(),
    };
    if hurt.read().count() > 0 {
        flashes.hurt = 0.6;
    }
    if hit.read().count() > 0 {
        flashes.hit = 0.7;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_hud(
    mut contexts: EguiContexts,
    state: Res<State<GameState>>,
    mut mobile: ResMut<TargetMobile>,
    mut radar_mode: ResMut<RadarMode>,
    mut next: ResMut<NextState<GameState>>,
    touch: Res<TouchControls>,
    started: Res<Started>,
    mut whistle: ResMut<TouchWhistle>,
    arena: Res<Layout>,
    flashes: Res<Flashes>,
    layout: Res<ScreenLayout>,
    shooter: Option<Single<(&Shooter, &Stunned)>>,
    target: Option<Single<(&Target, &Suspicion, Option<&Activity>)>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let screen = ctx.content_rect();
    let view = rect(layout.view);

    // Portrait deck under the view, holding the radar and whistle. Filled around the radar, not
    // under it: egui draws after every camera, so a full fill would paint over the radar.
    if layout.portrait() {
        let deck = rect(layout.deck);
        let painter = ctx.layer_painter(egui::LayerId::background());
        fill_around(&painter, deck, rect(layout.radar), egui::Color32::from_rgb(14, 20, 17));
        painter.hline(deck.x_range(), deck.top(), egui::Stroke::new(2.0, egui::Color32::from_rgb(40, 90, 60)));
        if touch.0 {
            painter.text(
                egui::pos2(deck.left() + deck.width() / 4.0, deck.center().y),
                egui::Align2::CENTER_CENTER,
                "drive",
                egui::FontId::proportional(18.0),
                egui::Color32::from_white_alpha(40),
            );
        }
    }

    egui::Area::new("status".into())
        .anchor(egui::Align2::LEFT_TOP, [16.0, 16.0])
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(240.0);
                if let Some(shooter) = &shooter {
                    let (shooter, stunned) = **shooter;
                    if stunned.left > 0.0 {
                        ui.colored_label(egui::Color32::from_rgb(255, 120, 60), "YOU (the shooter) · STUNNED");
                    } else {
                        ui.label("YOU (the shooter)");
                    }
                    ui.add(
                        egui::ProgressBar::new(shooter.hp / SHOOTER_MAX_HP)
                            .text(format!("{:.0} HP", shooter.hp))
                            .fill(egui::Color32::from_rgb(60, 170, 80)),
                    );
                }
                if let Some(target) = &target {
                    let (target, suspicion, activity) = **target;
                    ui.add_space(6.0);
                    ui.label(format!(
                        "TARGET  {}{}",
                        "♥".repeat(target.hp as usize),
                        "♡".repeat((TARGET_MAX_HP - target.hp) as usize)
                    ));
                    let colour = if suspicion.engaged {
                        egui::Color32::from_rgb(220, 50, 40)
                    } else {
                        egui::Color32::from_rgb(220, 170, 40)
                    };
                    ui.add(
                        egui::ProgressBar::new(suspicion.level)
                            .text(if suspicion.engaged { "ENGAGING" } else { "suspicion" })
                            .fill(colour),
                    );
                    let doing = match activity {
                        Some(Activity::Scanning) => "looking around",
                        Some(Activity::Wandering) => "wandering",
                        Some(Activity::Investigating) => "investigating a noise",
                        Some(Activity::TakingCover) => "RUNNING FOR COVER",
                        Some(Activity::Engaging) => "SHOOTING AT YOU",
                        None => "…",
                    };
                    ui.label(format!(
                        "he's {doing}{}",
                        if suspicion.sees_shooter { " · sees you" } else { "" }
                    ));
                }
                // No keyboard on a phone: the meta keys become buttons. They sit on the left
                // half, where a tap only wakes the drive stick, never fires.
                if touch.0 {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(format!("radar: {:?}", *radar_mode)).clicked() {
                            *radar_mode = radar_mode.next();
                        }
                        if ui.button(format!("he walks: {}", if mobile.0 { "on" } else { "off" })).clicked() {
                            mobile.0 = !mobile.0;
                        }
                    });
                }
            });
        });

    // Whistle for thumbs: a big round button on the right, just above the radar. Taps on it
    // don't fire (see `touch::TouchWhistle`).
    if touch.0 {
        let radar = layout.radar;
        let mut area = egui::Area::new("whistle".into());
        area = if layout.portrait() {
            // Centred over the radar in the deck.
            area.fixed_pos([radar.center().x - WHISTLE_SIZE / 2.0, radar.min.y - 16.0 - WHISTLE_SIZE])
        } else {
            let above_radar = if radar.height() > 0.0 { screen.height() - radar.min.y } else { 0.0 };
            area.anchor(egui::Align2::RIGHT_BOTTOM, [-24.0, -above_radar - 16.0])
        };
        let response = area
            .show(ctx, |ui| {
                // `add_sized` lays the button out centred-and-justified, so the label sits in the middle.
                ui.add_sized(
                    [WHISTLE_SIZE, WHISTLE_SIZE],
                    egui::Button::new(egui::RichText::new("whistle").size(18.0))
                        .corner_radius(WHISTLE_SIZE / 2.0)
                        .fill(egui::Color32::from_black_alpha(150))
                        .stroke(egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 220, 120))),
                )
            })
            .inner;
        if response.clicked() {
            whistle.pressed = true;
        }
        let r = response.rect;
        whistle.button = Rect::new(r.min.x, r.min.y, r.max.x, r.max.y);
    } else {
        whistle.button = Rect::default();
    }

    // Key hints, only in keyboard mode. Skip the area entirely in touch mode: drawn empty, egui
    // remembers it as zero-width and the hints wrap into a one-letter column when they come back.
    if !touch.0 {
        egui::Area::new("help".into())
            .anchor(egui::Align2::LEFT_BOTTOM, [16.0, -16.0])
            .show(ctx, |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "Up/Down move   Left/Right turn   Space fire   W whistle   M target walks: {}   Tab radar: {:?}   L arena: {}   F1 inspector",
                            if mobile.0 { "on" } else { "off" },
                            *radar_mode,
                            arena.name,
                        ))
                        .color(egui::Color32::WHITE)
                        .background_color(egui::Color32::from_black_alpha(140)),
                    )
                    .extend(),
                );
            });
    }

    // Frame + label for the radar viewport.
    if layout.radar.width() > 0.0 && *radar_mode != RadarMode::Off {
        let rect = rect(layout.radar);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, "radar".into()));
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 220, 120)),
            egui::StrokeKind::Outside,
        );
        painter.text(
            rect.left_top() + egui::vec2(6.0, 4.0),
            egui::Align2::LEFT_TOP,
            format!("RADAR · {:?}", *radar_mode).to_uppercase(),
            egui::FontId::monospace(14.0),
            egui::Color32::WHITE,
        );
    }

    // Flashes over the view.
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, "flash".into()));
    if flashes.hurt > 0.0 {
        painter.rect_filled(view, 0.0, egui::Color32::from_rgba_unmultiplied(255, 0, 0, (flashes.hurt * 120.0) as u8));
    }
    if flashes.hit > 0.0 {
        painter.rect_filled(view, 0.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, (flashes.hit * 60.0) as u8));
        hit_marker(&painter, view.center(), flashes.hit);
    }
    // His view reddens at the edges as he's wounded, pulsing on each hit.
    if let Some(target) = &target {
        let wounds = 1.0 - target.0.hp as f32 / TARGET_MAX_HP as f32;
        let strength = (wounds * 0.7 + flashes.hit * 0.5).min(1.0);
        if strength > 0.0 {
            let width = view.width().min(view.height()) * (0.12 + 0.18 * strength);
            vignette(&painter, view, width, egui::Color32::from_rgba_unmultiplied(150, 0, 0, (strength * 200.0) as u8));
        }
    }
    // He's dead: the view dims to dark red as he lies there.
    if *state.get() == GameState::Won {
        let dim = ((flashes.ended - 0.5) / 1.2).clamp(0.0, 1.0);
        painter.rect_filled(view, 0.0, egui::Color32::from_rgba_unmultiplied(40, 0, 0, (dim * 170.0) as u8));
    }

    let banner = match state.get() {
        GameState::Playing => None,
        _ if flashes.ended < BANNER_DELAY => None,
        GameState::Won => Some((flashes.banner, egui::Color32::from_rgb(90, 230, 110))),
        GameState::Lost => Some((flashes.banner, egui::Color32::from_rgb(240, 70, 60))),
    };
    if let Some((text, colour)) = banner {
        // Smaller on a phone in portrait, so the longer lines wrap to two rows, not four.
        let size = (screen.width() / 15.0).clamp(22.0, 36.0);
        egui::Area::new("banner".into())
            .anchor(egui::Align2::CENTER_CENTER, view.center() - screen.center())
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new(text).size(size).color(colour).strong());
                        if touch.0 {
                            if ui.button(egui::RichText::new("go again").size(22.0)).clicked() {
                                next.set(GameState::Playing);
                            }
                        } else {
                            ui.label(egui::RichText::new("press R to go again").size(18.0));
                        }
                    });
                });
            });
    }
    if !started.0 {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, "start dim".into()));
        painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(170));
        egui::Area::new("start".into())
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("SECOND PERSON SHOOTER").size(28.0).color(egui::Color32::WHITE).strong());
                    ui.label(egui::RichText::new("You see through his eyes. Hunt him down.").color(egui::Color32::LIGHT_GRAY));
                    ui.add_space(18.0);
                    ui.label(egui::RichText::new("tap to play").size(36.0).color(egui::Color32::from_rgb(90, 230, 110)).strong());
                    ui.label(egui::RichText::new("or press any key").size(18.0).color(egui::Color32::WHITE));
                    ui.add_space(18.0);
                    ui.label(egui::RichText::new("touch: left thumb drives · tap right to fire · whistle button").color(egui::Color32::LIGHT_GRAY));
                    ui.label(egui::RichText::new("keys: arrows drive · Space fires · W whistles").color(egui::Color32::LIGHT_GRAY));
                });
            });
    }
    Ok(())
}

fn rect(r: Rect) -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(r.min.x, r.min.y), egui::pos2(r.max.x, r.max.y))
}

/// Fill `outer` except for `hole`, which must lie inside it.
fn fill_around(painter: &egui::Painter, outer: egui::Rect, hole: egui::Rect, colour: egui::Color32) {
    let rects = [
        egui::Rect::from_x_y_ranges(outer.x_range(), outer.top()..=hole.top()),
        egui::Rect::from_x_y_ranges(outer.x_range(), hole.bottom()..=outer.bottom()),
        egui::Rect::from_x_y_ranges(outer.left()..=hole.left(), hole.y_range()),
        egui::Rect::from_x_y_ranges(hole.right()..=outer.right(), hole.y_range()),
    ];
    for r in rects {
        painter.rect_filled(r, 0.0, colour);
    }
}

/// An X of four short strokes around `at`, fading with `alpha`.
fn hit_marker(painter: &egui::Painter, at: egui::Pos2, alpha: f32) {
    let a = alpha.min(1.0);
    let fill = egui::Color32::from_rgba_unmultiplied(255, 60, 40, (a * 255.0) as u8);
    let edge = egui::Color32::from_rgba_unmultiplied(0, 0, 0, (a * 200.0) as u8);
    // Pops out a little as it fades.
    let (inner, outer) = (12.0 + 10.0 * (1.0 - a), 34.0 + 10.0 * (1.0 - a));
    for (x, y) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        let d = egui::vec2(x, y) * std::f32::consts::FRAC_1_SQRT_2;
        let stroke = [at + d * inner, at + d * outer];
        // Dark outline first so it reads on bright sky and dark walls alike.
        painter.line_segment(stroke, egui::Stroke::new(8.0, edge));
        painter.line_segment(stroke, egui::Stroke::new(4.5, fill));
    }
}

/// A border `width` thick that fades from `colour` at the edge to clear inside.
fn vignette(painter: &egui::Painter, rect: egui::Rect, width: f32, colour: egui::Color32) {
    let inner = rect.shrink(width);
    let outer = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    let inner = [inner.left_top(), inner.right_top(), inner.right_bottom(), inner.left_bottom()];
    let mut mesh = egui::Mesh::default();
    for i in 0..4 {
        mesh.colored_vertex(outer[i], colour);
        mesh.colored_vertex(inner[i], egui::Color32::TRANSPARENT);
    }
    for i in 0..4u32 {
        let j = (i + 1) % 4;
        mesh.add_triangle(2 * i, 2 * j, 2 * i + 1);
        mesh.add_triangle(2 * i + 1, 2 * j, 2 * j + 1);
    }
    painter.add(egui::Shape::mesh(mesh));
}
