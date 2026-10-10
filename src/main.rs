use bevy::prelude::*;
use bevy_egui::{EguiGlobalSettings, EguiPlugin};

fn main() {
    let mut app = App::new();
    app
        // The HUD camera owns the egui context, since game cameras get respawned per round.
        // Insert before EguiPlugin (it only init_resource's) so no camera ever races auto-creation.
        .insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..default()
        })
        .add_plugins((
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Second Person Shooter".into(),
                        // Web: render into the page's canvas and keep arrows/space from scrolling it.
                        canvas: Some("#game".into()),
                        fit_canvas_to_parent: true,
                        prevent_default_event_handling: true,
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    // We ship no .meta files. On the web each would cost an extra request, and dev
                    // servers (trunk) answer the missing .meta with index.html, which breaks the load.
                    meta_check: bevy::asset::AssetMetaCheck::Never,
                    ..default()
                }),
            EguiPlugin::default(),
            second_person::gameplay,
            second_person::presentation,
        ))
        // Every round's difficulty adapts to the player (headless tests keep the fixed tuning).
        .init_resource::<second_person::difficulty::AdaptiveDifficulty>();

    // F1 world inspector: opt-in via the `inspector` cargo feature (on in `dev`).
    #[cfg(feature = "inspector")]
    app.add_plugins(
        bevy_inspector_egui::quick::WorldInspectorPlugin::default()
            .run_if(bevy::input::common_conditions::input_toggle_active(false, KeyCode::F1)),
    );

    #[cfg(feature = "brp")]
    app.add_plugins(bevy_brp_extras::BrpExtrasPlugin);

    app.run();
}
