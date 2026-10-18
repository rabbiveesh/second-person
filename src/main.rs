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
                        title: "HIM · a second person shooter".into(),
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
        ));

    if let Some(url) = duel_server() {
        use rand::Rng;
        // Keep the connection ticking when the window isn't focused.
        app.insert_resource(bevy::winit::WinitSettings::continuous());
        let opts = second_person::duel::client::Options { id: rand::rng().random(), url, lag: 0, bot: false };
        second_person::duel::join(&mut app, opts);
    } else {
        app.add_plugins((second_person::gameplay, second_person::presentation))
            // Every round's difficulty adapts to the player (headless tests keep the fixed tuning).
            .init_resource::<second_person::difficulty::AdaptiveDifficulty>();
    }

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

/// The duel server to join, if asked: `--duel <host:port or URL>` natively, `?duel=` on the web.
/// Nothing in the game points here yet.
#[cfg(not(target_family = "wasm"))]
fn duel_server() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let spec = args.iter().position(|a| a == "--duel").and_then(|i| args.get(i + 1))?;
    Some(second_person::duel::server_url(spec, false))
}

#[cfg(target_family = "wasm")]
fn duel_server() -> Option<String> {
    let location = web_sys::window()?.location();
    let query = location.search().ok()?;
    let spec = query.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix("duel="))?;
    let secure = location.protocol().is_ok_and(|p| p == "https:");
    Some(second_person::duel::server_url(spec, secure))
}
