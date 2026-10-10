//! Debug dump: press F9 for a plain-text report of everything relevant: build info, every
//! reflected game resource (found through the type registry, so new ones show up for free),
//! the target's and shooter's components, the adaptive difficulty state and round history,
//! and a log of recent gameplay events. On the web it downloads as a `.txt`; on native it's
//! written to the working directory and the path is logged.

use std::collections::VecDeque;
use std::fmt::{Debug, Write as _};

use bevy::ecs::reflect::{ReflectComponent, ReflectResource};
use bevy::prelude::*;

use crate::{
    combat::{BulletImpact, GrappleFired, Gunshot, ShooterHit, TargetHit, WarningShot},
    radar::RadarMode,
    round::{GameState, TargetMobile},
    shooter::{Bump, Shooter, Whistle},
    target::Target,
};

pub fn plugin(app: &mut App) {
    app.init_resource::<EventLog>()
        .add_systems(Update, dump.run_if(f9_pressed));
    log_messages::<StateTransitionEvent<GameState>>(app);
    log_messages::<Gunshot>(app);
    log_messages::<BulletImpact>(app);
    log_messages::<TargetHit>(app);
    log_messages::<ShooterHit>(app);
    log_messages::<WarningShot>(app);
    log_messages::<GrappleFired>(app);
    log_messages::<Bump>(app);
    log_messages::<Whistle>(app);
}

/// How many recent gameplay events the dump includes.
const LOG_LEN: usize = 300;

/// Recent gameplay events: (seconds since startup, type, `Debug` text).
#[derive(Resource, Default)]
pub struct EventLog(VecDeque<(f32, &'static str, String)>);

fn log_messages<M: Message + Debug>(app: &mut App) {
    let name = std::any::type_name::<M>().rsplit("::").next().unwrap_or("?");
    app.add_systems(Last, move |mut r: MessageReader<M>, mut log: ResMut<EventLog>, time: Res<Time<Real>>| {
        for m in r.read() {
            if log.0.len() == LOG_LEN {
                log.0.pop_front();
            }
            log.0.push_back((time.elapsed_secs(), name, format!("{m:?}")));
        }
    });
}

fn f9_pressed(keys: Option<Res<ButtonInput<KeyCode>>>) -> bool {
    keys.is_some_and(|k| k.just_pressed(KeyCode::F9))
}

/// Build the whole report.
pub fn report(world: &mut World) -> String {
    let mut out = String::new();
    let secs = world.resource::<Time<Real>>().elapsed_secs();
    let _ = writeln!(out, "Second Person Shooter debug dump");
    let _ = writeln!(
        out,
        "version {} · commit {} · {} · uptime {secs:.1}s",
        env!("CARGO_PKG_VERSION"),
        option_env!("GITHUB_SHA").unwrap_or("local build"),
        if cfg!(target_arch = "wasm32") { "web" } else { "native" },
    );
    if let Some(state) = world.get_resource::<State<GameState>>() {
        let _ = writeln!(out, "game state: {:?}", state.get());
    }
    if let Some(mode) = world.get_resource::<RadarMode>() {
        let _ = writeln!(out, "radar: {mode:?}");
    }
    if let Some(mobile) = world.get_resource::<TargetMobile>() {
        let _ = writeln!(out, "target walks: {}", mobile.0);
    }

    section(&mut out, "adaptive difficulty");
    let _ = write!(out, "{}", crate::difficulty::debug_text(world));

    // Every reflected resource of ours.
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let mut ours: Vec<_> = registry.iter().filter(|r| r.type_info().type_path().starts_with("second_person::")).collect();
    ours.sort_by_key(|r| r.type_info().type_path());
    section(&mut out, "resources");
    // (Bevy 0.19 keeps resources as components on their own entities.)
    for reg in &ours {
        let tid = reg.type_id();
        if reg.data::<ReflectResource>().is_some()
            && let Some(cid) = world.components().get_valid_id(tid)
            && let Some(e) = world.resource_entities().get(cid)
            && let Ok(value) = world.get_reflect(e, tid)
        {
            let _ = writeln!(out, "{} = {:#?}", short(reg.type_info().type_path()), value.as_partial_reflect());
        }
    }

    // The target and the shooter: all their reflected components.
    for (title, entity) in [("target", single_with::<Target>(world)), ("shooter", single_with::<Shooter>(world))] {
        section(&mut out, title);
        let Some(e) = entity else {
            let _ = writeln!(out, "(not spawned)");
            continue;
        };
        for reg in registry.iter() {
            if reg.data::<ReflectComponent>().is_some()
                && let Ok(value) = world.get_reflect(e, reg.type_id())
            {
                let path = reg.type_info().type_path();
                if path.starts_with("second_person::") || path.ends_with("::Transform") {
                    let _ = writeln!(out, "{} = {:#?}", short(path), value.as_partial_reflect());
                }
            }
        }
    }
    drop(registry);

    section(&mut out, "recent events (oldest first)");
    for (t, name, text) in &world.resource::<EventLog>().0 {
        let _ = writeln!(out, "{t:9.2}s  {name:<22} {text}");
    }
    out
}

fn section(out: &mut String, title: &str) {
    let _ = writeln!(out, "\n=== {title} ===");
}

fn short(path: &str) -> &str {
    path.strip_prefix("second_person::").unwrap_or(path)
}

fn single_with<C: Component>(world: &mut World) -> Option<Entity> {
    world.query_filtered::<Entity, With<C>>().iter(world).next()
}

fn dump(world: &mut World) {
    let text = report(world);
    let secs = world.resource::<Time<Real>>().elapsed_secs() as u64;
    match save_report(&text, &format!("second-person-debug-{secs}.txt")) {
        Ok(where_) => info!("debug dump saved: {where_}"),
        Err(e) => warn!("debug dump failed: {e}"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn save_report(text: &str, name: &str) -> Result<String, String> {
    let p = std::env::current_dir().map_err(|e| e.to_string())?.join(name);
    std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(p.display().to_string())
}

/// Web: hand the report to the browser as a file download.
#[cfg(target_arch = "wasm32")]
fn save_report(text: &str, name: &str) -> Result<String, String> {
    use wasm_bindgen::JsCast;
    let err = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(text));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("text/plain");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts).map_err(err)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(err)?;
    let a: web_sys::HtmlAnchorElement =
        document.create_element("a").map_err(err)?.dyn_into().map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(name);
    a.click();
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(format!("downloaded {name}"))
}
