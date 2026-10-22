//! Experimental features: off in the normal game, switched on from a hidden panel (tap the HUD's
//! TARGET label five times; see `hud`). Each experiment is a `bool` field here, so the panel
//! lists it by reflection and the debug dump shows it with no extra wiring. On the web the
//! choices are kept in `localStorage`, so they survive a reload.

use bevy::prelude::*;

#[derive(Resource, Reflect, Default, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource)]
pub struct Experiments {
    /// His scramble dart (`scramble`), and the Mirror arrow that teaches it.
    pub scramble_dart: bool,
}

pub fn plugin(app: &mut App) {
    app.insert_resource(load()).add_systems(Last, save.run_if(resource_changed::<Experiments>));
}

/// Visit every experiment's name (spaced, for show) and switch, for the panel.
pub fn each(experiments: &mut Experiments, mut visit: impl FnMut(&str, &mut bool)) {
    use bevy::reflect::ReflectMut;
    let ReflectMut::Struct(fields) = experiments.reflect_mut() else {
        return;
    };
    for i in 0..fields.field_len() {
        let name = fields.name_at(i).unwrap_or("?").replace('_', " ");
        if let Some(on) = fields.field_at_mut(i).and_then(|f| f.try_downcast_mut::<bool>()) {
            visit(&name, on);
        }
    }
}

const KEY: &str = "him.experiments";

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// Saved as the names of the experiments that are on, comma separated.
fn load() -> Experiments {
    #[cfg_attr(not(target_arch = "wasm32"), allow(unused_mut))]
    let mut experiments = Experiments::default();
    #[cfg(target_arch = "wasm32")]
    if let Some(saved) = storage().and_then(|s| s.get_item(KEY).ok().flatten()) {
        let on: Vec<&str> = saved.split(',').collect();
        each(&mut experiments, |name, switch| *switch = on.contains(&name));
    }
    experiments
}

fn save(experiments: Res<Experiments>) {
    let mut copy = *experiments;
    let mut on = vec![];
    each(&mut copy, |name, switch| {
        if *switch {
            on.push(name.to_string());
        }
    });
    #[cfg(target_arch = "wasm32")]
    if let Some(storage) = storage() {
        let _ = storage.set_item(KEY, &on.join(","));
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (on, KEY);
}
