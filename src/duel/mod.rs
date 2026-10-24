//! The online 1v1 duel. Both players are shooters, and each sees the arena through the other's
//! eyes: your opponent is "him" on your screen, so the view, HUD, radar, fx and audio are the
//! single-player ones, fed by the server instead of by his AI.
//!
//! A dedicated server owns the match. Clients send only their inputs; the server moves both
//! bodies with the game's own movement (`shooter::Movement`), fires, and decides every hit, with
//! the same bullet magnetism single player has. Each client predicts its own body by re-running
//! that movement (rolling back when the server disagrees) and shows the opponent, whose head is
//! your camera, interpolated a little in the past so the view never snaps.
//!
//! Hidden for now: `cargo run --bin duel -- server` hosts a match, and the game joins one when
//! started with `--duel <host:port or ws(s):// URL>`, or on the web with `?duel=` in the page URL.
//! `cargo run --bin duel -- bot` adds a headless bot player.

pub mod client;
#[cfg(not(target_family = "wasm"))]
pub mod server;

#[cfg(test)]
mod arch;

use std::{net::SocketAddr, time::Duration};

use avian3d::{
    collider_tree::ColliderTrees,
    dynamics::solver::islands::{IslandPlugin, IslandSleepingPlugin},
    physics_transform::PhysicsTransformPlugin,
    prelude::*,
};
use bevy::{ecs::component::ComponentId, prelude::*};
use lightyear::{
    avian3d::{plugin::AvianReplicationMode, prelude::LightyearAvianPlugin},
    input::{config::InputConfig, leafwing::prelude::InputPlugin},
    prelude::*,
};
use serde::{Deserialize, Serialize};

use crate::{
    arena::Layout,
    combat::{self, HitKind},
    round::{self, Online},
    shooter::{self, Bearings, ControlScheme, Scrambled, ShooterAction, Sidestep, Stagger, Stunned},
};

pub const TICK_HZ: f64 = 60.0;
pub const PORT: u16 = 5888;
const PROTOCOL_ID: u64 = 0x4849_4D31;
/// Not a secret: it only keeps strays from other netcode games out. Matchmaking will issue real
/// connect tokens.
const KEY: [u8; 32] = [7; 32];

pub fn tick() -> Duration {
    Duration::from_secs_f64(1.0 / TICK_HZ)
}

// ---------------------------------------------------------------------------
// Protocol
// ---------------------------------------------------------------------------

/// Which connection a body belongs to.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PlayerId(pub PeerId);

/// A player's health, as the server has it. Clients show it on their own `Shooter` or on the
/// opponent's `Target` (only one of each may exist, so the components themselves aren't sent).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Health(pub f32);

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Score(pub u32);

/// This round's arena. Generation is deterministic, so a seed is the whole arena on the wire.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ArenaSeed(pub u64);

/// Hold to shut your eyes, for a while, then they need a rest. Shut eyes blind your opponent,
/// who sees through them, and you can't shoot.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Eyelids {
    pub closed: bool,
    /// Ticks until they can close again (0 = ready).
    pub cooldown: u32,
}

/// What the server tells clients happened, for their fx, audio and HUD.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NetEvent {
    /// Someone fired (sent to everyone but the shooter, who already showed it).
    Shot { by: PeerId, muzzle: Vec3, dir: Vec3 },
    Hit { victim: PeerId, from: Vec3, at: Vec3, damage: f32, kind: HitKind },
    Impact { at: Vec3 },
    Kill { winner: PeerId },
}

pub struct Events;

/// What both sides register, in the same order.
pub fn protocol(app: &mut App) {
    app.add_plugins(InputPlugin::<ShooterAction> {
        config: InputConfig {
            // Send our interpolation delay with inputs, so the server can lag-compensate shots.
            lag_compensation: true,
            ..default()
        },
    });
    app.component::<PlayerId>().replicate();
    app.component::<Health>().replicate();
    app.component::<Score>().replicate();
    app.component::<ArenaSeed>().replicate();
    app.component::<Eyelids>().replicate();
    sim_component::<Stagger>(app);
    sim_component::<Stunned>(app);
    sim_component::<Sidestep>(app);
    sim_component::<Scrambled>(app);
    app.register_message::<NetEvent>().add_direction(NetworkDirection::ServerToClient);
    app.add_channel::<Events>(ChannelSettings {
        mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
        ..default()
    })
    .add_direction(NetworkDirection::ServerToClient);
}

/// The game, minus everything that only single player has (him, his AI, local rounds).
pub fn gameplay(app: &mut App) {
    app.insert_resource(Online)
        // The duel steers tank-style: the other schemes steer by things only you can see.
        .insert_resource(ControlScheme::Tank)
        .add_plugins(
            PhysicsPlugins::default()
                .build()
                // lightyear syncs transforms, and rolls back what it must.
                .disable::<PhysicsTransformPlugin>()
                .disable::<PhysicsInterpolationPlugin>()
                .disable::<IslandPlugin>()
                .disable::<IslandSleepingPlugin>(),
        )
        .add_plugins(LightyearAvianPlugin {
            replication_mode: AvianReplicationMode::Position { sync_to_transform: false },
            rollback_resources: true,
            ..default()
        })
        .add_plugins((
            round::plugin,
            crate::arena::plugin,
            shooter::plugin,
            crate::difficulty::plugin,
            combat::common,
            crate::scramble::messages,
            crate::experiments::plugin,
        ))
        .add_observer(place_by_transform)
        .configure_sets(FixedUpdate, shooter::Movement.in_set(SimSet));
    // What lightyear's avian plugin predicts and rolls back, and the rest of what movement may read.
    for id in [
        app.world_mut().register_component::<Position>(),
        app.world_mut().register_component::<Rotation>(),
        app.world_mut().register_component::<LinearVelocity>(),
        app.world_mut().register_component::<AngularVelocity>(),
        app.world_mut().register_component::<ColliderAabb>(),
        app.world_mut().register_component::<ColliderTrees>(),
    ] {
        registry(app).state.push(id);
    }
    round_constant::<Layout>(app);
    round_constant::<ControlScheme>(app);
    round_constant::<Bearings>(app);
    // Shapes never change mid-round; in `FixedUpdate`, `Time` is the fixed step.
    for id in [app.world_mut().register_component::<Collider>(), app.world_mut().register_component::<Time>()] {
        registry(app).round_constants.push(id);
    }
}

/// lightyear syncs physics to transforms one way only, so a body spawned at a `Transform` (the
/// arena's walls and floor, a joining player) would start at the origin. Take its place from the
/// transform once, as it's added.
fn place_by_transform(
    t: On<Add, RigidBody>,
    mut q: Query<(&Transform, &mut Position, &mut Rotation), Without<ChildOf>>,
) {
    if let Ok((transform, mut pos, mut rot)) = q.get_mut(t.entity)
        && pos.0 == Vec3::ZERO
    {
        pos.0 = transform.translation;
        rot.0 = transform.rotation;
    }
}

// ---------------------------------------------------------------------------
// The simulation contract
//
// A client predicts its own body by re-running the server's simulation, so the simulation must
// be a pure function of predicted state and inputs:
//
// - Predicted state is registered through `sim_component` (or by lightyear's avian plugin).
// - `SimSet` (FixedUpdate, both sides, re-run on rollback) advances it, reading only predicted
//   state, inputs and round constants.
// - `AuthoritySet` (server only) overrules it: damage, death, respawn. Clients pick that up as a
//   misprediction and roll back.
// - Round constants (the arena, the control scheme) only change between rounds: in a state
//   transition, or in `AuthoritySet` / `FollowServerSet` (clients applying what the server said).
//
// The `arch` tests walk every schedule's system access and fail on any breach.
// ---------------------------------------------------------------------------

pub trait Simulated:
    Component<Mutability = bevy::ecs::component::Mutable> + Clone + PartialEq + std::fmt::Debug
{
}
impl Simulated for Stagger {}
impl Simulated for Stunned {}
impl Simulated for Sidestep {}
impl Simulated for Scrambled {}

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SimSet;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct AuthoritySet;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FollowServerSet;

/// Predicted state, and what the simulation may read besides it.
#[derive(Resource, Default)]
pub struct SimRegistry {
    pub state: Vec<ComponentId>,
    pub round_constants: Vec<ComponentId>,
}

fn registry(app: &mut App) -> Mut<'_, SimRegistry> {
    app.world_mut().get_resource_or_init::<SimRegistry>()
}

fn round_constant<T: Resource>(app: &mut App) {
    let id = app.world_mut().register_component::<T>();
    registry(app).round_constants.push(id);
}

fn sim_component<T>(app: &mut App)
where
    T: Simulated + Serialize + serde::de::DeserializeOwned,
{
    let id = app.world_mut().register_component::<T>();
    registry(app).state.push(id);
    app.component::<T>().replicate().predict();
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Simulated network conditions, for feeling latency on one machine.
pub fn conditioner(rtt_ms: u64) -> Option<RecvLinkConditioner> {
    (rtt_ms > 0).then(|| {
        RecvLinkConditioner::new(LinkConditionerConfig {
            incoming_latency: Duration::from_millis(rtt_ms / 2),
            incoming_jitter: Duration::from_millis(rtt_ms / 20),
            ..LinkConditionerConfig::average_condition()
        })
    })
}

/// `host:port` or `ip:port` (port optional) to an IPv4 address.
pub fn resolve(spec: &str) -> Result<SocketAddr, String> {
    use std::net::ToSocketAddrs;
    let spec = if spec.contains(':') { spec.to_string() } else { format!("{spec}:{PORT}") };
    spec.to_socket_addrs()
        .map_err(|e| format!("can't resolve {spec}: {e}"))?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| format!("{spec} has no IPv4 address"))
}

/// The server's WebSocket URL from what a player typed: a full `ws://` or `wss://` URL as is, or a
/// bare `host[:port]`, which gets `wss://` when `secure` (a page served over https may only open
/// secure sockets) and the default port when it's plain.
pub fn server_url(spec: &str, secure: bool) -> String {
    if spec.starts_with("ws://") || spec.starts_with("wss://") {
        return spec.to_string();
    }
    match (secure, spec.contains(':')) {
        (true, _) => format!("wss://{spec}"),
        (false, true) => format!("ws://{spec}"),
        (false, false) => format!("ws://{spec}:{PORT}"),
    }
}

/// Where a muzzle is and which way it points, for a body at `at` facing `rot`.
pub fn muzzle(at: Vec3, rot: Quat) -> (Vec3, Vec3) {
    (at + rot * Vec3::new(0.3, 0.15, -0.9), rot * Vec3::NEG_Z)
}

// ---------------------------------------------------------------------------
// Apps
// ---------------------------------------------------------------------------

/// No window, no renderer: the server, and bots playing headless. The arena still builds its
/// meshes (they carry the colliders), so assets and meshes are in.
pub fn headless(app: &mut App, frame: Duration) {
    app.add_plugins((
        MinimalPlugins.set(bevy::app::ScheduleRunnerPlugin::run_loop(frame)),
        bevy::log::LogPlugin::default(),
        bevy::state::app::StatesPlugin,
        bevy::input::InputPlugin,
        AssetPlugin::default(),
        TransformPlugin,
        bevy::mesh::MeshPlugin,
    ))
    .init_asset::<StandardMaterial>();
}

#[cfg(not(target_family = "wasm"))]
pub fn server_app(lag: u64) -> App {
    let mut app = App::new();
    let frame = std::env::var("DUEL_LOOP_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(16);
    headless(&mut app, Duration::from_millis(frame));
    app.add_plugins(lightyear::prelude::server::ServerPlugins { tick_duration: tick() })
        // Before the game's plugins: the shooter's input plugin would otherwise read a keyboard.
        .add_plugins(leafwing_input_manager::plugin::InputManagerPlugin::<ShooterAction>::server());
    protocol(&mut app);
    gameplay(&mut app);
    server::plugin(&mut app, lag);
    app
}

/// Join a duel instead of playing single player: on top of a windowed app (the game's own
/// window and egui), the networking, the game and its presentation.
pub fn join(app: &mut App, opts: client::Options) {
    app.add_plugins(lightyear::prelude::client::ClientPlugins { tick_duration: tick() });
    protocol(app);
    gameplay(app);
    // No title screen or round intro: the server starts the rounds, and the intro pauses time.
    app.insert_resource(crate::start::Started(true))
        .insert_resource(crate::intro::IntroEnabled(false))
        .add_plugins(crate::presentation);
    client::plugin(app, opts);
}

/// A headless player for self-play and tests (`opts.bot` to have it play).
pub fn headless_client_app(opts: client::Options) -> App {
    let mut app = App::new();
    headless(&mut app, tick());
    app.add_plugins(lightyear::prelude::client::ClientPlugins { tick_duration: tick() });
    protocol(&mut app);
    gameplay(&mut app);
    // What presentation would have provided that the client's own systems use.
    app.init_resource::<crate::hud::Notes>();
    client::plugin(&mut app, opts);
    app
}
