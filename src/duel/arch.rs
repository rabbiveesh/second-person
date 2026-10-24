//! The simulation contract, checked: every system in both apps, by set membership and the data it
//! touches. Systems are initialized, never run, so nothing opens a socket.

use std::collections::HashSet;

use avian3d::prelude::*;
use bevy::ecs::{
    component::ComponentId,
    query::ComponentAccessKind,
    schedule::{InternedSystemSet, NodeId, SystemKey, graph::Direction},
    system::System,
};
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;

use super::*;
use crate::shooter::{Shooter, ShooterAction, Stagger};

/// One system's place in the contract and what it touches.
struct Sys {
    name: String,
    schedule: String,
    sim: bool,
    authority: bool,
    follow: bool,
    reads: Vec<ComponentId>,
    writes: Vec<ComponentId>,
}

fn client_app() -> App {
    headless_client_app(client::Options { id: 1, url: server_url("127.0.0.1", false), lag: 0, bot: true })
}

/// Every system we wrote, in every schedule, with its set membership and data access.
/// Systems are initialized but never run, so nothing binds a socket.
fn survey(app: &mut App) -> Vec<Sys> {
    let world = app.world_mut();
    let mut schedules = world.remove_resource::<Schedules>().unwrap();
    let mut out = vec![];
    for (label, schedule) in schedules.iter_mut() {
        let graph = schedule.graph_mut();
        let members = |set: InternedSystemSet| -> HashSet<SystemKey> {
            let mut found = HashSet::new();
            let Some(root) = graph.system_sets.get_key(set) else { return found };
            let mut stack = vec![NodeId::Set(root)];
            while let Some(n) = stack.pop() {
                for child in graph.hierarchy().graph().neighbors_directed(n, Direction::Outgoing) {
                    match child {
                        NodeId::System(k) => {
                            found.insert(k);
                        }
                        set => stack.push(set),
                    }
                }
            }
            found
        };
        let sim = members(SimSet.intern());
        let authority = members(AuthoritySet.intern());
        let follow = members(FollowServerSet.intern());
        let keys: Vec<SystemKey> = graph.systems.iter().map(|(k, ..)| k).collect();
        for key in keys {
            let system = graph.systems.get_mut(key).unwrap();
            let name = system.name().to_string();
            if !name.starts_with("second_person::") {
                continue; // engine / lightyear internals (rollback, interpolation) are trusted
            }
            let access = System::initialize(system, world);
            let mut sys = Sys {
                name,
                schedule: format!("{label:?}"),
                sim: sim.contains(&key),
                authority: authority.contains(&key),
                follow: follow.contains(&key),
                reads: vec![],
                writes: vec![],
            };
            let all = access.combined_access().try_iter_access().unwrap_or_else(|_| {
                panic!("{} has unbounded world access (exclusive system?)", sys.name)
            });
            for kind in all {
                match kind {
                    ComponentAccessKind::Shared(id) => sys.reads.push(id),
                    ComponentAccessKind::Exclusive(id) => sys.writes.push(id),
                    ComponentAccessKind::Archetypal(_) => {} // With/Without/Has: no data flows
                }
            }
            out.push(sys);
        }
    }
    world.insert_resource(schedules);
    out
}

/// The contract, as a list of human-readable breaches.
fn violations(app: &mut App, server: bool) -> Vec<String> {
    let systems = survey(app);
    let world = app.world();
    let registry = world.resource::<SimRegistry>();
    let state: HashSet<ComponentId> = registry.state.iter().copied().collect();
    let constants: HashSet<ComponentId> = registry.round_constants.iter().copied().collect();
    let inputs = world.components().component_id::<ActionState<ShooterAction>>().unwrap();
    let name = |id: ComponentId| {
        world.components().get_name(id).map(|n| n.shortname().to_string()).unwrap_or_default()
    };
    let mut v = vec![];
    for s in &systems {
        let who = format!("{} ({})", s.name, s.schedule);
        if s.sim {
            if s.schedule != "FixedUpdate" {
                v.push(format!("{who} is in SimSet but not FixedUpdate, so rollback won't re-run it"));
            }
            for &id in &s.reads {
                if !state.contains(&id) && !constants.contains(&id) && id != inputs {
                    v.push(format!(
                        "{who} is simulation but depends on {}, which isn't predicted: \
                         make it Simulated or move the logic out of SimSet",
                        name(id)
                    ));
                }
            }
            for &id in &s.writes {
                if !state.contains(&id) {
                    v.push(format!("{who} is simulation but writes {}, which isn't predicted", name(id)));
                }
            }
        }
        if s.authority && !server {
            v.push(format!("{who}: AuthoritySet is server-only"));
        }
        // State transitions happen between rounds, while no body moves.
        let between_rounds = s.schedule.starts_with("OnEnter") || s.schedule.starts_with("OnExit");
        if !s.authority && !s.follow && !between_rounds {
            for &id in &s.writes {
                if constants.contains(&id) {
                    v.push(format!(
                        "{who} changes round constant {} outside AuthoritySet/FollowServerSet: \
                         the sim reads it, so only the server may decide it",
                        name(id)
                    ));
                }
            }
        }
        if !s.sim && !s.authority {
            for &id in &s.writes {
                if state.contains(&id) {
                    v.push(format!(
                        "{who} writes predicted {} outside SimSet/AuthoritySet: \
                         the client can't reproduce it, so it will mispredict",
                        name(id)
                    ));
                }
            }
        }
    }
    v
}

fn assert_clean(v: Vec<String>) {
    assert!(v.is_empty(), "simulation contract broken:\n  {}", v.join("\n  "));
}

#[test]
fn server_respects_simulation_contract() {
    assert_clean(violations(&mut server_app(0), true));
}

#[test]
fn client_respects_simulation_contract() {
    assert_clean(violations(&mut client_app(), false));
}

#[test]
fn sim_runs_on_both_sides() {
    for (mut app, side) in [(server_app(0), "server"), (client_app(), "client")] {
        let names: Vec<String> = survey(&mut app).into_iter().filter(|s| s.sim).map(|s| s.name).collect();
        for wanted in ["shooter::drive", "shooter::bump"] {
            assert!(names.iter().any(|n| n.ends_with(wanted)), "{side} doesn't run {wanted}: {names:?}");
        }
    }
}

// The checker itself: the bugs it exists to catch.

fn knocked_down_when_hurt(mut q: Query<(&mut LinearVelocity, &Shooter)>) {
    for (mut vel, shooter) in &mut q {
        if shooter.hp < 50.0 {
            vel.0 = Vec3::ZERO;
        }
    }
}

fn shoved(mut q: Query<&mut Stagger>) {
    for mut s in &mut q {
        s.left = 1.0;
    }
}

fn moves_walls(mut layout: ResMut<Layout>) {
    layout.blocks.clear();
}

fn decides_hits(mut q: Query<&mut Shooter>) {
    for mut s in &mut q {
        s.hp = 0.0;
    }
}

#[test]
fn catches_known_mispredictions() {
    let mut app = client_app();
    app.add_systems(FixedUpdate, (knocked_down_when_hurt.in_set(SimSet), shoved, moves_walls))
        .add_systems(Update, (knocked_down_when_hurt.in_set(SimSet), decides_hits.in_set(AuthoritySet)));
    let v = violations(&mut app, false);
    let hit = |needle: &str| v.iter().any(|m| m.contains(needle));
    assert!(hit("knocked_down_when_hurt") && hit("depends on Shooter"), "{v:#?}");
    assert!(hit("shoved") && hit("outside SimSet"), "{v:#?}");
    assert!(hit("not FixedUpdate"), "{v:#?}");
    assert!(hit("moves_walls") && hit("Layout"), "{v:#?}");
    assert!(hit("decides_hits") && hit("server-only"), "{v:#?}");
}

/// Presentation runs on the duel client too, and a system reading a message or resource nobody
/// registered panics on its first frame (the browser build just dies). So the client must have
/// every resource single player's gameplay registers, messages included, except the ones listed
/// here as single player only. Adding to gameplay means deciding which side it goes on.
#[test]
fn client_has_what_presentation_reads() {
    // Matched as substrings of the type name.
    const SINGLE_PLAYER_ONLY: &[&str] = &[
        // Physics features the duel turns off: lightyear owns sleeping and interpolation.
        "avian3d::dynamics::solver::islands",
        "avian3d::dynamics::rigid_body::sleeping",
        "bevy_transform_interpolation::",
        // His AI: behaviour tree and navmesh.
        "bevior_tree::",
        "vleue_navigator::",
        // His dart, thrown only in single player. Its messages are shared (`scramble::messages`).
        "second_person::scramble::Armed",
        "second_person::scramble::Cooldown",
    ];
    let names = |app: &App| -> HashSet<String> {
        app.world().iter_resources().map(|(info, _)| info.name().to_string()).collect()
    };
    let mut single = App::new();
    headless(&mut single, tick());
    crate::gameplay(&mut single);
    single.finish();
    let mut duel = client_app();
    duel.finish();
    let mut missing: Vec<_> = names(&single)
        .difference(&names(&duel))
        .filter(|n| !SINGLE_PLAYER_ONLY.iter().any(|only| n.contains(only)))
        .cloned()
        .collect();
    missing.sort();
    assert!(missing.is_empty(), "the duel client lacks:\n  {}", missing.join("\n  "));
}
