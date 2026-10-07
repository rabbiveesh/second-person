//! HUD drawn with egui on a dedicated overlay camera.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, PrimaryEguiContext, egui};

use crate::{
    combat::{ShooterHit, TargetHit},
    radar::{RadarMode, RadarRect},
    round::{GameState, TargetMobile},
    shooter::{SHOOTER_MAX_HP, Shooter, Stunned},
    target::{Activity, Suspicion, TARGET_MAX_HP, Target},
    touch::{TouchControls, TouchWhistle},
};

/// Screen flashes: red when you're hit, white when the target is hit.
#[derive(Resource, Default)]
struct Flashes {
    hurt: f32,
    hit: f32,
}

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
    mut flashes: ResMut<Flashes>,
    mut hurt: MessageReader<ShooterHit>,
    mut hit: MessageReader<TargetHit>,
) {
    let decay = time.delta_secs() * 2.5;
    flashes.hurt = (flashes.hurt - decay).max(0.0);
    flashes.hit = (flashes.hit - decay).max(0.0);
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
    mut whistle: ResMut<TouchWhistle>,
    flashes: Res<Flashes>,
    radar: Res<RadarRect>,
    shooter: Option<Single<(&Shooter, &Stunned)>>,
    target: Option<Single<(&Target, &Suspicion, Option<&Activity>)>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

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
                        if ui.button("whistle").clicked() {
                            whistle.0 = true;
                        }
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

    egui::Area::new("help".into())
        .anchor(egui::Align2::LEFT_BOTTOM, [16.0, -16.0])
        .show(ctx, |ui| {
            if touch.0 {
                return;
            }
            ui.label(
                egui::RichText::new(format!(
                    "Up/Down move   Left/Right turn   Space fire   W whistle   M target walks: {}   Tab radar: {:?}   F1 inspector",
                    if mobile.0 { "on" } else { "off" },
                    *radar_mode
                ))
                .color(egui::Color32::WHITE)
                .background_color(egui::Color32::from_black_alpha(140)),
            );
        });

    // Frame + label for the radar viewport.
    let r = radar.0;
    if r.width() > 0.0 && *radar_mode != RadarMode::Off {
        let rect = egui::Rect::from_min_max(egui::pos2(r.min.x, r.min.y), egui::pos2(r.max.x, r.max.y));
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

    // Full-screen flashes.
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, "flash".into()));
    if flashes.hurt > 0.0 {
        painter.rect_filled(screen, 0.0, egui::Color32::from_rgba_unmultiplied(255, 0, 0, (flashes.hurt * 120.0) as u8));
    }
    if flashes.hit > 0.0 {
        painter.rect_filled(screen, 0.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, (flashes.hit * 160.0) as u8));
    }

    let banner = match state.get() {
        GameState::Playing => None,
        GameState::Won => Some(("TARGET DOWN", egui::Color32::from_rgb(90, 230, 110))),
        GameState::Lost => Some(("YOU WERE SPOTTED. AND SHOT.", egui::Color32::from_rgb(240, 70, 60))),
    };
    if let Some((text, colour)) = banner {
        egui::Area::new("banner".into())
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new(text).size(36.0).color(colour).strong());
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
    Ok(())
}
