//! THROWAWAY network spike: 1v1 "mirror duel" over a dedicated authoritative server.
//! Each player drives their own body (tank controls) but sees through the OTHER player's eyes.
//!
//! Not production code: shortcuts everywhere, nothing here is meant to be merged or kept.
//!
//!   cargo run --example net_spike -- server
//!   cargo run --example net_spike -- client --id 1 [--lag 120] [--bot] [--headless]
//!   cargo run --example net_spike -- client --id 2 --bot --headless   (Claude's bot)
//!
//! What it exercises:
//! - server authority: clients only send inputs; server moves bodies, fires, decides hits
//! - client prediction + rollback for your own body (lightyear `.predict()` + `.add_correction()`)
//! - interpolation for the opponent's body, which is also your camera (no camera snapping)
//! - lag compensation for projectiles: the server tests bullets against where the victim was
//!   in the shooter's (interpolated) view, capped at MAX_LAG_COMP_TICKS
//! - link conditioner (`--lag <rtt ms>`) to feel it on localhost
//! - simulation contract (`SimSet`/`AuthoritySet`), enforced by `cargo test --example net_spike`
//! - liveness: the server exits when nobody is playing (`MATCH_*_S` env overrides)

use std::{collections::VecDeque, net::SocketAddr, time::Duration};

use bevy::{
    camera::{ScalingMode, Viewport, visibility::RenderLayers},
    ecs::entity::MapEntities,
    math::curve::{Ease, FunctionCurve, Interval},
    prelude::*,
};
use lightyear::{
    input::native::prelude::*,
    interpolation::plugin::InterpolationDelay,
    netcode::{NetcodeClient, NetcodeServer},
    prelude::{client::*, server::*, *},
};
use lightyear::input::config::InputConfig;
use lightyear::input::client::InputSystems;

use lightyear::netcode::client_plugin::NetcodeConfig;
use lightyear::prelude::input::native::InputMarker;
use rand::Rng;
use second_person::arena::{self, ARENA_HALF};
use serde::{Deserialize, Serialize};

const TICK_HZ: f64 = 60.0;
const DT: f32 = 1.0 / TICK_HZ as f32;
const PORT: u16 = 5888;
const PROTOCOL_ID: u64 = 0x5EC0_4D;
const KEY: [u8; 32] = [7; 32];

const RADIUS: f32 = 0.35;
const MOVE_SPEED: f32 = 5.0;
const TURN_SPEED: f32 = 2.4;
const EYE_HEIGHT: f32 = 1.6;
const BULLET_SPEED: f32 = 45.0;
const BULLET_HEIGHT: f32 = 1.3;
const FIRE_COOLDOWN_TICKS: u32 = 18;
const DAMAGE: f32 = 25.0;
const MAX_HP: f32 = 100.0;
const MAX_LAG_COMP_TICKS: u32 = 15; // 250 ms
const RESPAWN_TICKS: u32 = 180;
const BLINK_MAX_TICKS: u32 = 90; // eyes stay shut at most 1.5 s per press
const BLINK_COOLDOWN_TICKS: u32 = 240; // 4 s after they open again

// ---------------------------------------------------------------- protocol

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct PlayerId(PeerId);

/// Body pose on the ground plane. Yaw 0 faces -Z.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
struct Pose {
    pos: Vec2,
    yaw: f32,
}

impl Ease for Pose {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| Pose {
            pos: start.pos.lerp(end.pos, t),
            yaw: start.yaw + angle_diff(end.yaw, start.yaw) * t,
        })
    }
}

// Needed by `.add_correction()` (smooths out rollback corrections of your own body).
impl Diffable for Pose {
    fn base_value() -> Self {
        Pose::default()
    }
    fn diff(&self, new: &Self) -> Self {
        Pose { pos: new.pos - self.pos, yaw: angle_diff(new.yaw, self.yaw) }
    }
    fn apply_diff(&mut self, d: &Self) {
        self.pos += d.pos;
        self.yaw += d.yaw;
    }
}

/// Whether this body may act this tick. Dead or between rounds = false. Predicted, because
/// movement reads it: the server decides it, the client learns it through rollback.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct Active(bool);

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct Hp(f32);

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct Score(u32);

/// Server-owned eyelid state, replicated to both clients.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
struct Eyelids {
    closed: bool,
    /// Ticks until the eyes may close again (0 = ready).
    cooldown: u32,
}

/// Server-simulated projectile, replicated (interpolated) to the player who didn't fire it.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct BulletPos(Vec2);

impl Ease for BulletPos {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| BulletPos(start.0.lerp(end.0, t)))
    }
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone, Reflect)]
struct Inputs {
    fwd: i8,
    turn: i8,
    fire: bool,
    /// Hold to close your eyes: blacks out the opponent's screen (it IS your eyes).
    blink: bool,
}

impl MapEntities for Inputs {
    fn map_entities<M: EntityMapper>(&mut self, _: &mut M) {}
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
enum NetEvent {
    Hit { victim: PeerId, hp: f32, lag_ticks: u32 },
    Impact { at: Vec2 },
    Kill { winner: PeerId },
}

struct Reliable;

fn protocol(app: &mut App) {
    app.add_plugins(InputPlugin::<Inputs> {
        config: InputConfig::<Inputs> {
            // send our interpolation delay with inputs so the server can lag-compensate
            lag_compensation: true,
            ..default()
        },
    });
    app.component::<PlayerId>().replicate();
    sim_component::<Pose>(app).add_linear_interpolation().add_correction();
    sim_component::<Active>(app);
    app.component::<Hp>().replicate();
    app.component::<Score>().replicate();
    app.component::<Eyelids>().replicate();
    app.component::<BulletPos>().replicate().add_linear_interpolation();
    app.register_message::<NetEvent>()
        .add_direction(NetworkDirection::ServerToClient);
    app.add_channel::<Reliable>(ChannelSettings {
        mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
        ..default()
    })
    .add_direction(NetworkDirection::ServerToClient);
}

// ---------------------------------------------------------------- simulation contract
//
// The client predicts its own body by re-running the server's simulation. That only works if
// the simulation is a pure function of predicted state + inputs. The contract:
//
// - Predicted state implements `Simulated` and is registered through `sim_component`, which
//   turns on prediction (the trait bounds are what rollback needs, so it won't compile otherwise).
// - `SimSet` (FixedUpdate, both apps, re-run on rollback) advances it, reading only predicted
//   state and inputs.
// - `AuthoritySet` (server only) may overrule it: damage, death, respawn. Clients pick those
//   up as a misprediction and roll back.
//
// `arch` tests at the bottom walk every schedule's system access and fail on any breach.

trait Simulated: Component<Mutability = bevy::ecs::component::Mutable> + Clone + PartialEq + std::fmt::Debug {}
impl Simulated for Pose {}
impl Simulated for Active {}

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
struct SimSet;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
struct AuthoritySet;

/// Components registered as predicted simulation state, plus what the sim may read besides them.
#[derive(Resource, Default)]
struct SimRegistry {
    state: Vec<bevy::ecs::component::ComponentId>,
}

fn sim_component<T>(app: &mut App) -> lightyear::prediction::registry::PredictedComponentRegistration<'_, T>
where
    T: Simulated + Serialize + serde::de::DeserializeOwned,
{
    let id = app.world_mut().register_component::<T>();
    app.world_mut().get_resource_or_init::<SimRegistry>().state.push(id);
    app.component::<T>().replicate().predict()
}

/// The one movement system: the server runs it authoritatively, the client runs it for its own
/// (predicted) body and re-runs it during rollback. Interpolated opponents are only displayed.
fn sim_move(mut q: Query<(&mut Pose, &ActionState<Inputs>, &Active), Without<Interpolated>>) {
    for (mut pose, input, active) in &mut q {
        if active.0 {
            step(&mut pose, &input.0);
        }
    }
}

// ---------------------------------------------------------------- shared sim

fn dir(yaw: f32) -> Vec2 {
    Vec2::new(-yaw.sin(), -yaw.cos())
}

fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// Deterministic tank movement against the static arena: the same code runs on the server
/// (authoritative) and on the client (prediction), so mispredictions only come from latency.
fn step(pose: &mut Pose, i: &Inputs) {
    pose.yaw += i.turn.signum() as f32 * TURN_SPEED * DT;
    let d = dir(pose.yaw) * i.fwd.signum() as f32 * MOVE_SPEED * DT;
    if arena::is_clear(pose.pos + Vec2::new(d.x, 0.0), RADIUS) {
        pose.pos.x += d.x;
    }
    if arena::is_clear(pose.pos + Vec2::new(0.0, d.y), RADIUS) {
        pose.pos.y += d.y;
    }
}

fn random_spawn(other: Option<Vec2>) -> Pose {
    let mut rng = rand::rng();
    loop {
        let p = Vec2::new(
            rng.random_range(-ARENA_HALF + 3.0..ARENA_HALF - 3.0),
            rng.random_range(-ARENA_HALF + 3.0..ARENA_HALF - 3.0),
        );
        if arena::is_clear(p, 1.0) && other.is_none_or(|o| o.distance(p) > 25.0) {
            return Pose { pos: p, yaw: rng.random_range(0.0..std::f32::consts::TAU) };
        }
    }
}

// ---------------------------------------------------------------- main

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn conditioner(rtt_ms: u64) -> Option<RecvLinkConditioner> {
    (rtt_ms > 0).then(|| {
        RecvLinkConditioner::new(LinkConditionerConfig {
            incoming_latency: Duration::from_millis(rtt_ms / 2),
            incoming_jitter: Duration::from_millis(rtt_ms / 20),
            ..LinkConditionerConfig::average_condition()
        })
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tick = Duration::from_secs_f64(1.0 / TICK_HZ);
    let lag: u64 = arg(&args, "--lag").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut app = App::new();
    match args.get(1).map(String::as_str) {
        Some("server") => {
            app.add_plugins((
                MinimalPlugins.set(bevy::app::ScheduleRunnerPlugin::run_loop(Duration::from_millis(
                    std::env::var("SPIKE_LOOP_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(2),
                ))),
                bevy::log::LogPlugin::default(),
                bevy::state::app::StatesPlugin,
            ));
            app.add_plugins(ServerPlugins { tick_duration: tick });
            protocol(&mut app);
            server_plugin(&mut app, lag);
        }
        Some("client") => {
            let id: u64 = arg(&args, "--id").and_then(|s| s.parse().ok()).unwrap_or_else(|| rand::rng().random());
            let server: SocketAddr = arg(&args, "--server")
                .unwrap_or(format!("127.0.0.1:{PORT}"))
                .parse()
                .expect("--server ip:port");
            let bot = args.iter().any(|a| a == "--bot");
            let headless = args.iter().any(|a| a == "--headless");
            if headless {
                app.add_plugins((
                    MinimalPlugins.set(bevy::app::ScheduleRunnerPlugin::run_loop(tick)),
                    bevy::log::LogPlugin::default(),
                    TransformPlugin,
                    bevy::input::InputPlugin,
                    bevy::state::app::StatesPlugin,
                ));
            } else {
                app.add_plugins(DefaultPlugins.set(WindowPlugin {
                    primary_window: Some(Window {
                        title: format!("net spike: player {id}"),
                        resolution: (960, 600).into(),
                        ..default()
                    }),
                    ..default()
                }));
                app.insert_resource(bevy::winit::WinitSettings::continuous());
            }
            app.add_plugins(ClientPlugins { tick_duration: tick });
            protocol(&mut app);
            client_plugin(&mut app, id, server, lag, bot, headless);
        }
        _ => {
            eprintln!("usage: net_spike server | client --id N [--lag ms] [--bot] [--headless] [--server ip:port]");
            return;
        }
    }
    app.run();
}

// ---------------------------------------------------------------- server

#[derive(Component)]
struct Gun {
    cooldown: u32,
}

#[derive(Component, Default)]
struct History(VecDeque<(Tick, Vec2)>);

#[derive(Component)]
struct Bullet {
    owner: PeerId,
    dir: Vec2,
    lag_ticks: u32,
    life: u32,
}

#[derive(Resource, Default)]
struct RoundState {
    respawn_in: Option<u32>,
}

fn server_plugin(app: &mut App, lag: u64) {
    app.insert_resource(ReplicationMetadata::new(Duration::from_millis(33)))
        .init_resource::<RoundState>()
        .add_systems(Startup, move |mut commands: Commands| {
            let server = commands
                .spawn((
                    Name::new("Server"),
                    Server::new(conditioner(lag)),
                    NetcodeServer::new(lightyear::netcode::server_plugin::NetcodeConfig {
                        protocol_id: PROTOCOL_ID,
                        private_key: KEY,
                        ..default()
                    }),
                    LocalAddr(SocketAddr::from(([0, 0, 0, 0], PORT))),
                    ServerUdpIo::default(),
                ))
                .id();
            commands.trigger(Start { entity: server });
            info!("server listening on udp :{PORT}");
        })
        .add_observer(|t: On<Add, LinkOf>, mut commands: Commands| {
            commands.entity(t.entity).insert(ReplicationSender);
        })
        .add_observer(on_connected)
        .add_systems(
            FixedUpdate,
            (
                sim_move.in_set(SimSet),
                record_history,
                server_eyelids,
                server_fire,
                (server_bullets, server_round).chain().in_set(AuthoritySet),
            )
                .chain(),
        )
        .init_resource::<Liveness>()
        .add_systems(Update, (server_log, liveness));
}

// ---------------------------------------------------------------- liveness
//
// A match server costs money for as long as it runs, so it must end itself. Exiting the process
// stops the Fly Machine (restart policy "no"). Each limit can be overridden with an env var.

fn env_secs(name: &str, default: f32) -> f32 {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[derive(Resource)]
struct Liveness {
    /// Waiting for a second player (at start, or after one left).
    wait_for_players: f32,
    /// Neither player has touched a control.
    afk: f32,
    /// Everyone left; a short grace for reconnects.
    empty: f32,
    /// Ceiling no matter what, in case the other checks have a bug.
    max_match: f32,
    /// Last time both players were in / somebody pressed something / anyone was connected.
    last_full: f32,
    last_input: f32,
    last_connected: Option<f32>,
}

impl Default for Liveness {
    fn default() -> Self {
        Self {
            wait_for_players: env_secs("MATCH_WAIT_S", 120.0),
            afk: env_secs("MATCH_AFK_S", 60.0),
            empty: env_secs("MATCH_EMPTY_S", 10.0),
            max_match: env_secs("MATCH_MAX_S", 900.0),
            last_full: 0.0,
            last_input: 0.0,
            last_connected: None,
        }
    }
}

fn liveness(
    time: Res<Time<Real>>,
    mut l: ResMut<Liveness>,
    clients: Query<(), (With<ClientOf>, With<Connected>)>,
    players: Query<&ActionState<Inputs>, With<PlayerId>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    let connected = clients.iter().count();
    if connected > 0 {
        l.last_connected = Some(now);
    }
    // The AFK clock only runs while both players are in.
    if connected < 2 || players.iter().any(|a| a.0 != Inputs::default()) {
        l.last_input = now;
    }
    if connected >= 2 {
        l.last_full = now;
    }
    let reason = if l.last_connected.is_some_and(|t| now - t > l.empty) {
        Some("everyone left")
    } else if now - l.last_full > l.wait_for_players {
        Some("no opponent")
    } else if now - l.last_input > l.afk {
        Some("nobody is playing")
    } else if now > l.max_match {
        Some("match hit the time limit")
    } else {
        None
    };
    if let Some(reason) = reason {
        info!("LIVENESS: shutting down after {now:.0}s: {reason}");
        exit.write(AppExit::Success);
    }
}

fn on_connected(
    t: On<Add, Connected>,
    remote: Query<&RemoteId, With<ClientOf>>,
    players: Query<&Pose, With<PlayerId>>,
    mut commands: Commands,
) {
    let Ok(id) = remote.get(t.entity) else { return };
    let id = id.0;
    if players.iter().count() >= 2 {
        warn!("third client {id:?} ignored: 1v1 only");
        return;
    }
    let pose = random_spawn(players.iter().next().map(|p| p.pos));
    info!("player {id:?} joined at {:?}", pose.pos);
    commands.spawn((
        PlayerId(id),
        pose,
        Active(true),
        Hp(MAX_HP),
        Score(0),
        Eyelids::default(),
        BlinkTimer(0),
        Gun { cooldown: 0 },
        History::default(),
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::Single(id)),
        InterpolationTarget::to_clients(NetworkTarget::AllExceptSingle(id)),
        ControlledBy { owner: t.entity, lifetime: Default::default() },
    ));
}

#[derive(Component)]
struct BlinkTimer(u32);

/// Eyes close while blink is held, for at most BLINK_MAX_TICKS, then a cooldown.
fn server_eyelids(mut q: Query<(&ActionState<Inputs>, &mut Eyelids, &mut BlinkTimer, &Hp)>) {
    for (input, mut lids, mut shut_for, hp) in &mut q {
        let mut l = *lids;
        if l.closed {
            shut_for.0 += 1;
            if !input.0.blink || shut_for.0 >= BLINK_MAX_TICKS || hp.0 <= 0.0 {
                l.closed = false;
                l.cooldown = BLINK_COOLDOWN_TICKS;
            }
        } else if l.cooldown > 0 {
            l.cooldown -= 1;
        } else if input.0.blink && hp.0 > 0.0 {
            info!("BLINK: a player closed their eyes");
            l.closed = true;
            shut_for.0 = 0;
        }
        // Only touch the component on real changes (cooldown ticks every frame would spam replication).
        if l.closed != lids.closed || (l.cooldown == 0) != (lids.cooldown == 0) {
            *lids = l;
        } else {
            lids.bypass_change_detection().cooldown = l.cooldown;
        }
    }
}

fn record_history(timeline: Res<LocalTimeline>, mut q: Query<(&Pose, &mut History)>) {
    let tick = timeline.tick();
    for (pose, mut h) in &mut q {
        h.0.push_back((tick, pose.pos));
        if h.0.len() > 64 {
            h.0.pop_front();
        }
    }
}

fn server_fire(
    round: Res<RoundState>,
    mut commands: Commands,
    mut q: Query<(&PlayerId, &Pose, &ActionState<Inputs>, &mut Gun, &ControlledBy, &Hp, &Eyelids)>,
    delays: Query<&InterpolationDelay, With<ClientOf>>,
) {
    for (id, pose, input, mut gun, controlled, hp, lids) in &mut q {
        gun.cooldown = gun.cooldown.saturating_sub(1);
        // You can't shoot with your eyes shut.
        if !input.0.fire || lids.closed || gun.cooldown > 0 || hp.0 <= 0.0 || round.respawn_in.is_some() {
            continue;
        }
        gun.cooldown = FIRE_COOLDOWN_TICKS;
        // How far in the past the shooter saw the world (their interpolation delay).
        let lag_ticks = delays
            .get(controlled.owner)
            .map(|d| d.delay.tick_diff() as u32)
            .unwrap_or(0)
            .min(MAX_LAG_COMP_TICKS);
        let d = dir(pose.yaw);
        commands.spawn((
            Bullet { owner: id.0, dir: d, lag_ticks, life: 90 },
            BulletPos(pose.pos + d * (RADIUS + 0.2)),
            Replicate::to_clients(NetworkTarget::AllExceptSingle(id.0)),
            InterpolationTarget::to_clients(NetworkTarget::AllExceptSingle(id.0)),
        ));
    }
}

fn server_bullets(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    mut bullets: Query<(Entity, &mut Bullet, &mut BulletPos)>,
    mut players: Query<(&PlayerId, &History, &mut Hp, &Pose)>,
    mut scores: Query<(&PlayerId, &mut Score)>,
    mut round: ResMut<RoundState>,
    mut sender: ServerMultiMessageSender,
    server: Single<&Server>,
) {
    let now = timeline.tick();
    let mut events = vec![];
    for (e, mut b, mut bp) in &mut bullets {
        b.life = b.life.saturating_sub(1);
        let from = bp.0;
        let to = from + b.dir * BULLET_SPEED * DT;
        let mut done = b.life == 0;
        for k in 1..=4 {
            let p = from.lerp(to, k as f32 / 4.0);
            // Lag compensation: test against where the victim was in the shooter's view.
            for (pid, hist, mut hp, pose) in &mut players {
                if pid.0 == b.owner || hp.0 <= 0.0 {
                    continue;
                }
                let then = now - b.lag_ticks;
                let victim = hist
                    .0
                    .iter()
                    .rev()
                    .find(|(t, _)| *t <= then)
                    .map(|(_, p)| *p)
                    .unwrap_or(pose.pos);
                if p.distance(victim) < RADIUS + 0.1 {
                    hp.0 = (hp.0 - DAMAGE).max(0.0);
                    info!(
                        "HIT {:?} -> {:?} hp {} (lag comp {} ticks, rewound miss-by-now {:.2}m)",
                        b.owner, pid.0, hp.0, b.lag_ticks, p.distance(pose.pos)
                    );
                    events.push(NetEvent::Hit { victim: pid.0, hp: hp.0, lag_ticks: b.lag_ticks });
                    if hp.0 <= 0.0 {
                        events.push(NetEvent::Kill { winner: b.owner });
                        round.respawn_in = Some(RESPAWN_TICKS);
                        for (sid, mut s) in &mut scores {
                            if sid.0 == b.owner {
                                s.0 += 1;
                                info!("KILL by {:?}, score {}", b.owner, s.0);
                            }
                        }
                    }
                    done = true;
                }
            }
            if done {
                break;
            }
            if !arena::is_clear(p, 0.05) {
                events.push(NetEvent::Impact { at: p });
                done = true;
                break;
            }
        }
        bp.0 = to;
        if done {
            commands.entity(e).despawn();
        }
    }
    for ev in events {
        let _ = sender.send::<_, Reliable>(&ev, &server, &NetworkTarget::All);
    }
}

fn server_log(time: Res<Time>, mut t: Local<f32>, q: Query<(&PlayerId, &Pose, &Hp, &ActionState<Inputs>)>) {
    *t += time.delta_secs();
    if *t < 5.0 {
        return;
    }
    *t = 0.0;
    for (id, p, hp, i) in &q {
        info!("server: {:?} at {:?} hp {} input {:?}", id.0, p.pos, hp.0, i.0);
    }
}

/// Freezes both bodies between a kill and the next round, then respawns them.
fn server_round(mut round: ResMut<RoundState>, mut q: Query<(&mut Pose, &mut Hp, &mut Active)>) {
    let Some(n) = round.respawn_in.as_mut() else { return };
    *n -= 1;
    for (_, _, mut active) in &mut q {
        active.set_if_neq(Active(false));
    }
    if *n > 0 {
        return;
    }
    round.respawn_in = None;
    let mut first = None;
    for (mut pose, mut hp, mut active) in &mut q {
        *pose = random_spawn(first);
        first = Some(pose.pos);
        hp.0 = MAX_HP;
        *active = Active(true);
    }
    info!("new round");
}

// ---------------------------------------------------------------- client

#[derive(Resource)]
struct BotBrain {
    enabled: bool,
    /// Ticks left holding blink.
    blink_for: u32,
    /// Where the bot last "saw" you: it loses track while you have your eyes shut.
    last_seen: Option<Vec2>,
    aim_error: f32,
    retarget: u32,
}

#[derive(Component)]
struct LocalBullet {
    vel: Vec2,
    life: f32,
}

#[derive(Component)]
struct Eyes;

#[derive(Component)]
struct Radar;

#[derive(Component)]
struct Flash(f32);

#[derive(Resource, Default)]
struct Hud {
    my_hp: f32,
    flash: f32,
    msg: String,
    msg_t: f32,
}

fn client_plugin(app: &mut App, id: u64, server: SocketAddr, lag: u64, bot: bool, headless: bool) {
    // Not added by ClientPlugins: without it, rollback/correction systems silently never run.
    app.insert_resource(PredictionManager::default());
    app.insert_resource(BotBrain { enabled: bot, blink_for: 0, last_seen: None, aim_error: 0.0, retarget: 0 })
        .init_resource::<Hud>()
        .add_systems(Startup, move |mut commands: Commands| {
            let auth = Authentication::Manual {
                server_addr: server,
                client_id: id,
                private_key: KEY,
                protocol_id: PROTOCOL_ID,
            };
            let client = commands
                .spawn((
                    Name::new("Client"),
                    Client,
                    ReplicationReceiver,
                    Link::default().with_conditioner(conditioner(lag)),
                    LocalAddr(SocketAddr::from(([0, 0, 0, 0], 0))),
                    PeerAddr(server),
                    NetcodeClient::new(auth, NetcodeConfig { client_timeout_secs: 3, token_expire_secs: -1, ..default() })
                        .expect("netcode"),
                    UdpIo::default(),
                ))
                .id();
            commands.trigger(Connect { entity: client });
            info!("client {id} connecting to {server} (simulated rtt {lag} ms)");
        })
        .add_observer(|t: On<Add, Controlled>, q: Query<(), With<PlayerId>>, mut commands: Commands| {
            if q.contains(t.entity) {
                commands.entity(t.entity).insert(InputMarker::<Inputs>::default());
            }
        })
        .add_systems(FixedPreUpdate, write_inputs.in_set(InputSystems::WriteClientInputs))
        .add_systems(FixedUpdate, sim_move.in_set(SimSet))
        .add_systems(Update, (receive_events, log_state));
    if !headless {
        app.add_systems(Startup, setup_view).add_plugins(arena::plugin);
        client_view_systems(app);
    }
}

fn client_view_systems(app: &mut App) {
    app.add_systems(FixedUpdate, local_fire_fx.after(SimSet).run_if(not(is_in_rollback)))
        .add_systems(
            Update,
            (dress_players, dress_bullets, sync_transforms, eyes_follow_opponent, tick_fx, hud_title, shots),
        );
}

/// Your inputs: arrows + space, or the bot. The bot "cheats" the way any client could:
/// it knows the opponent's position because it has to, to render their eyes.
fn write_inputs(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut me: Query<(&mut ActionState<Inputs>, &Pose, Option<&Eyelids>), With<InputMarker<Inputs>>>,
    them: Query<(&Pose, Option<&Eyelids>), (With<Interpolated>, With<PlayerId>)>,
    mut brain: ResMut<BotBrain>,
) {
    let Ok((mut action, me, my_lids)) = me.single_mut() else { return };
    let mut i = Inputs::default();
    if brain.enabled {
        if let Ok((them_now, their_lids)) = them.single() {
            let mut rng = rand::rng();
            // Play fair-ish: when the opponent shuts their eyes, the bot's "screen" is black too,
            // so it keeps acting on where it last saw them.
            if !their_lids.is_some_and(|l| l.closed) || brain.last_seen.is_none() {
                brain.last_seen = Some(them_now.pos);
            }
            let them = &Pose { pos: brain.last_seen.unwrap(), yaw: them_now.yaw };
            // Close our eyes when they're facing us down a clear line: blind them, then reposition.
            let to_me = me.pos - them_now.pos;
            let they_aim_at_me = angle_diff((-to_me.x).atan2(-to_me.y), them_now.yaw).abs() < 0.3
                && !arena::los_blocked(me.pos, them_now.pos);
            if brain.blink_for == 0
                && they_aim_at_me
                && my_lids.is_some_and(|l| !l.closed && l.cooldown == 0)
                && rng.random_bool(0.04)
            {
                brain.blink_for = rng.random_range(40..90);
            }
            if brain.blink_for > 0 {
                brain.blink_for -= 1;
                i.blink = true;
            }
            if brain.retarget == 0 {
                brain.aim_error = rng.random_range(-0.12..0.12);
                brain.retarget = rng.random_range(20..60);
            }
            brain.retarget -= 1;
            let to = them.pos - me.pos;
            let clear_shot = !arena::los_blocked(me.pos, them.pos);
            // No line of sight: flank around the cover instead of staring at it.
            let flank = if clear_shot { 0.0 } else { 1.1 };
            let want = (-to.x).atan2(-to.y) + brain.aim_error + flank;
            let err = angle_diff(want, me.yaw);
            i.turn = if err > 0.04 { 1 } else if err < -0.04 { -1 } else { 0 };
            let ahead = me.pos + dir(me.yaw) * 1.2;
            if !arena::is_clear(ahead, RADIUS) {
                i.turn = 1;
                i.fwd = -1;
            } else if to.length() > 9.0 || !clear_shot {
                i.fwd = 1;
            } else if to.length() < 5.0 {
                i.fwd = -1;
            }
            i.fire = !i.blink && clear_shot && err.abs() < 0.15 && to.length() < 35.0;
            if i.blink {
                // eyes shut: break line of sight instead of standing there
                i.fwd = 1;
                i.turn = 1;
            }
        }
    } else if let Some(k) = keys {
        i.fwd = k.pressed(KeyCode::ArrowUp) as i8 - k.pressed(KeyCode::ArrowDown) as i8;
        i.turn = k.pressed(KeyCode::ArrowLeft) as i8 - k.pressed(KeyCode::ArrowRight) as i8;
        i.fire = k.pressed(KeyCode::Space);
        i.blink = k.pressed(KeyCode::ShiftLeft) || k.pressed(KeyCode::KeyC);
    }
    action.0 = i;
}

fn receive_events(
    mut rx: Query<&mut MessageReceiver<NetEvent>>,
    me: Query<&PlayerId, With<Predicted>>,
    mut hud: ResMut<Hud>,
    mut commands: Commands,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut mats: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let me = me.single().ok().map(|p| p.0);
    for mut r in &mut rx {
        for ev in r.receive() {
            match ev {
                NetEvent::Hit { victim, hp, lag_ticks } => {
                    let mine = Some(victim) == me;
                    info!("{} hp {hp} (server lag-comp {lag_ticks} ticks)", if mine { "I was HIT" } else { "I HIT them" });
                    if mine {
                        hud.flash = 0.35;
                    }
                }
                NetEvent::Kill { winner } => {
                    hud.msg = if Some(winner) == me { "YOU WIN the round".into() } else { "YOU DIED".into() };
                    hud.msg_t = 3.0;
                    info!("{}", hud.msg);
                }
                NetEvent::Impact { at } => {
                    if let (Some(m), Some(mt)) = (meshes.as_mut(), mats.as_mut()) {
                        commands.spawn((
                            Mesh3d(m.add(Sphere::new(0.15))),
                            MeshMaterial3d(mt.add(StandardMaterial { base_color: Color::srgb(1.0, 0.9, 0.4), unlit: true, ..default() })),
                            Transform::from_xyz(at.x, BULLET_HEIGHT, at.y),
                            Flash(0.25),
                        ));
                    }
                }
            }
        }
    }
}

fn log_state(
    time: Res<Time>,
    mut t: Local<f32>,
    me: Query<(&Pose, &Hp, &Score), With<Predicted>>,
    them: Query<(&Pose, &Score), (With<Interpolated>, With<PlayerId>)>,
) {
    *t += time.delta_secs();
    if *t < 5.0 {
        return;
    }
    *t = 0.0;
    if let (Ok((p, hp, s)), Ok((o, os))) = (me.single(), them.single()) {
        info!("me {:?} hp {} | them {:?} | score {}-{}", p.pos, hp.0, o.pos, s.0, os.0);
    }
}

// ---------------------------------------------------------------- client visuals

fn setup_view(mut commands: Commands) {
    // Main view: the opponent's eyes (moved every frame by eyes_follow_opponent).
    commands.spawn((
        Eyes,
        Camera3d::default(),
        Camera { order: 0, ..default() },
        Projection::Perspective(PerspectiveProjection { fov: 70f32.to_radians(), ..default() }),
        Transform::from_xyz(0.0, 50.0, 30.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // Radar (Full mode, spike shortcut): top-down ortho in the corner.
    commands.spawn((
        Radar,
        Camera3d::default(),
        Camera {
            order: 1,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.02, 0.07, 0.04)),
            viewport: Some(Viewport {
                physical_position: UVec2::new(960 - 210, 600 - 210),
                physical_size: UVec2::splat(200),
                ..default()
            }),
            ..default()
        },
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical { viewport_height: ARENA_HALF * 2.0 + 2.0 },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0.0, 60.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
        RenderLayers::from_layers(&[0, 1]),
    ));
}

fn dress_players(
    mut commands: Commands,
    q: Query<(Entity, Has<Predicted>), (With<PlayerId>, Without<Mesh3d>, Or<(With<Predicted>, With<Interpolated>)>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    for (e, mine) in &q {
        let color = if mine { Color::srgb(0.2, 0.9, 0.3) } else { Color::srgb(0.9, 0.3, 0.2) };
        let blip = if mine { Color::srgb(0.1, 1.0, 1.0) } else { Color::srgb(1.0, 0.15, 0.15) };
        let body = meshes.add(Capsule3d::new(RADIUS, 1.1));
        commands.entity(e).try_insert((
            Mesh3d(body),
            MeshMaterial3d(mats.add(color)),
            Transform::default(),
            Visibility::default(),
            children![(
                // a "gun" so you can tell which way a body faces
                Mesh3d(meshes.add(Cuboid::new(0.12, 0.12, 0.6))),
                MeshMaterial3d(mats.add(Color::srgb(0.1, 0.1, 0.1))),
                Transform::from_xyz(0.25, 0.3, -0.4),
            ), (
                // big radar blip, on a layer only the radar camera renders
                Mesh3d(meshes.add(Sphere::new(1.4))),
                MeshMaterial3d(mats.add(StandardMaterial { base_color: blip, unlit: true, ..default() })),
                Transform::from_xyz(0.0, 4.0, 0.0),
                RenderLayers::layer(1),
            )],
        ));
    }
}

fn dress_bullets(
    mut commands: Commands,
    q: Query<Entity, (With<BulletPos>, With<Interpolated>, Without<Mesh3d>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    for e in &q {
        commands.entity(e).try_insert((
            Mesh3d(meshes.add(Sphere::new(0.12))),
            MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::srgb(1.0, 0.4, 0.1), unlit: true, ..default() })),
            Transform::default(),
            Visibility::default(),
        ));
    }
}

fn sync_transforms(
    mut players: Query<(&Pose, &mut Transform), (With<PlayerId>, Or<(With<Predicted>, With<Interpolated>)>)>,
    mut bullets: Query<(&BulletPos, &mut Transform), (Without<PlayerId>, With<Interpolated>)>,
) {
    for (p, mut t) in &mut players {
        t.translation = Vec3::new(p.pos.x, 0.9, p.pos.y);
        t.rotation = Quat::from_rotation_y(p.yaw);
    }
    for (b, mut t) in &mut bullets {
        t.translation = Vec3::new(b.0.x, BULLET_HEIGHT, b.0.y);
    }
}

/// The whole concept: your camera is the opponent's head. Their pose is interpolated (smooth,
/// ~100 ms behind), so the view you look through never snaps, even when your own predicted
/// body gets corrected.
fn eyes_follow_opponent(
    mut commands: Commands,
    them: Query<(&Pose, Option<&Eyelids>), (With<Interpolated>, With<PlayerId>)>,
    eyes: Single<(Entity, &mut Transform, &mut Camera), (With<Eyes>, Without<Radar>)>,
    radar: Single<(Entity, &mut Camera), With<Radar>>,
) {
    let (cam_entity, mut eyes, mut camera) = eyes.into_inner();
    let (radar_entity, mut radar_cam) = radar.into_inner();
    if let Ok((p, lids)) = them.single() {
        // They closed their eyes: you're blind. Render nothing, on black.
        let shut = lids.is_some_and(|l| l.closed);
        let layers = if shut { RenderLayers::layer(7) } else { RenderLayers::layer(0) };
        commands.entity(cam_entity).insert(layers);
        camera.clear_color = if shut { ClearColorConfig::Custom(Color::BLACK) } else { ClearColorConfig::Default };
        // Blind means blind: the radar goes dark too.
        let radar_layers = if shut { RenderLayers::layer(7) } else { RenderLayers::from_layers(&[0, 1]) };
        commands.entity(radar_entity).insert(radar_layers);
        radar_cam.clear_color =
            ClearColorConfig::Custom(if shut { Color::BLACK } else { Color::srgb(0.02, 0.07, 0.04) });
        eyes.translation = Vec3::new(p.pos.x, EYE_HEIGHT, p.pos.y) + dir(p.yaw).extend(0.0).xzy() * 0.36;
        eyes.rotation = Quat::from_rotation_y(p.yaw);
    }
}

/// Instant feedback for your own shots: a local cosmetic bullet from the predicted pose.
/// The server's real bullet is never sent back to you; hits come back as events.
fn local_fire_fx(
    mut commands: Commands,
    q: Query<(&Pose, &ActionState<Inputs>, &Hp, &Eyelids), With<Predicted>>,
    mut cooldown: Local<u32>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    *cooldown = cooldown.saturating_sub(1);
    let Ok((pose, input, hp, lids)) = q.single() else { return };
    if !input.0.fire || input.0.blink || lids.closed || *cooldown > 0 || hp.0 <= 0.0 {
        return;
    }
    *cooldown = FIRE_COOLDOWN_TICKS;
    let d = dir(pose.yaw);
    let p = pose.pos + d * (RADIUS + 0.2);
    commands.spawn((
        LocalBullet { vel: d * BULLET_SPEED, life: 2.0 },
        Mesh3d(meshes.add(Sphere::new(0.12))),
        MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::srgb(0.4, 1.0, 0.5), unlit: true, ..default() })),
        Transform::from_xyz(p.x, BULLET_HEIGHT, p.y),
    ));
}

fn tick_fx(
    time: Res<Time>,
    mut commands: Commands,
    mut bullets: Query<(Entity, &mut LocalBullet, &mut Transform)>,
    mut flashes: Query<(Entity, &mut Flash)>,
) {
    let dt = time.delta_secs();
    for (e, mut b, mut t) in &mut bullets {
        b.life -= dt;
        t.translation += Vec3::new(b.vel.x, 0.0, b.vel.y) * dt;
        if b.life <= 0.0 || !arena::is_clear(t.translation.xz(), 0.05) {
            commands.entity(e).despawn();
        }
    }
    for (e, mut f) in &mut flashes {
        f.0 -= dt;
        if f.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

/// Spike HUD: the window title, plus the sky turning red when you get hit.
fn hud_title(
    time: Res<Time>,
    mut hud: ResMut<Hud>,
    me: Query<(&Hp, &Score, &Eyelids), With<Predicted>>,
    them: Query<(&Score, &Eyelids), (With<Interpolated>, With<PlayerId>)>,
    mut window: Single<&mut Window>,
    mut clear: ResMut<ClearColor>,
) {
    let dt = time.delta_secs();
    hud.flash = (hud.flash - dt).max(0.0);
    hud.msg_t = (hud.msg_t - dt).max(0.0);
    let title = match (me.single(), them.single()) {
        (Ok((hp, s, lids)), Ok((os, their_lids))) => {
            hud.my_hp = hp.0;
            let eyes = if lids.closed {
                "YOUR EYES ARE SHUT (they're blind, you can't shoot)".to_string()
            } else if lids.cooldown > 0 {
                "eyes recovering".to_string()
            } else {
                "shift: close your eyes".to_string()
            };
            let status = if hud.msg_t > 0.0 {
                hud.msg.clone()
            } else if their_lids.closed {
                "THEY CLOSED THEIR EYES".into()
            } else {
                "arrows move, space fires, you see through THEIR eyes".into()
            };
            format!("HP {:.0}   score {} - {}   [{eyes}]   {status}", hp.0, s.0, os.0)
        }
        _ => "waiting for an opponent...".into(),
    };
    if window.title != title {
        window.title = title;
    }
    clear.0 = Color::srgb(0.55, 0.7, 0.85).mix(&Color::srgb(0.9, 0.1, 0.1), (hud.flash / 0.35).min(1.0));
}

/// Dev aid: `SPIKE_SHOT=/some/dir` saves a screenshot every 3 s (used to check the view headlessly).
fn shots(time: Res<Time>, mut t: Local<f32>, mut n: Local<u32>, mut commands: Commands) {
    let Ok(dir) = std::env::var("SPIKE_SHOT") else { return };
    *t += time.delta_secs();
    if *t < 3.0 || *n >= 8 {
        return;
    }
    *t = 0.0;
    *n += 1;
    use bevy::render::view::screenshot::{Screenshot, save_to_disk};
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(format!("{dir}/shot{}.png", *n)));
}

// ---------------------------------------------------------------- architecture tests

#[cfg(test)]
mod arch {
    use super::*;
    use bevy::ecs::{
        component::ComponentId,
        query::ComponentAccessKind,
        schedule::{InternedSystemSet, NodeId, SystemKey, graph::Direction},
        system::System,
    };
    use std::collections::HashSet;

    /// One system's place in the contract and what it touches.
    struct Sys {
        name: String,
        schedule: String,
        sim: bool,
        authority: bool,
        reads: Vec<ComponentId>,
        writes: Vec<ComponentId>,
    }

    fn server_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin));
        app.add_plugins(ServerPlugins { tick_duration: Duration::from_secs_f64(1.0 / TICK_HZ) });
        protocol(&mut app);
        server_plugin(&mut app, 0);
        app
    }

    fn client_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            bevy::input::InputPlugin,
            bevy::state::app::StatesPlugin,
        ));
        app.add_plugins(ClientPlugins { tick_duration: Duration::from_secs_f64(1.0 / TICK_HZ) });
        protocol(&mut app);
        client_plugin(&mut app, 1, SocketAddr::from(([127, 0, 0, 1], PORT)), 0, false, true);
        // The windowed client's systems too, minus the parts that need a renderer to build.
        client_view_systems(&mut app);
        app
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
            let keys: Vec<SystemKey> = graph.systems.iter().map(|(k, ..)| k).collect();
            for key in keys {
                let system = graph.systems.get_mut(key).unwrap();
                let name = system.name().to_string();
                if !name.starts_with("net_spike::") {
                    continue; // engine / lightyear internals (rollback, interpolation) are trusted
                }
                let access = System::initialize(system, world);
                let mut sys = Sys {
                    name,
                    schedule: format!("{label:?}"),
                    sim: sim.contains(&key),
                    authority: authority.contains(&key),
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
        let state: HashSet<ComponentId> = world.resource::<SimRegistry>().state.iter().copied().collect();
        let inputs = world.components().component_id::<ActionState<Inputs>>().unwrap();
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
                for &id in s.reads.iter().chain(&s.writes) {
                    if !state.contains(&id) && id != inputs {
                        v.push(format!(
                            "{who} is simulation but depends on {}, which isn't predicted: \
                             make it Simulated or move the logic out of SimSet",
                            name(id)
                        ));
                    }
                }
            }
            if s.authority && !server {
                v.push(format!("{who}: AuthoritySet is server-only"));
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
        assert_clean(violations(&mut server_app(), true));
    }

    #[test]
    fn client_respects_simulation_contract() {
        assert_clean(violations(&mut client_app(), false));
    }

    #[test]
    fn sim_runs_on_both_sides() {
        for (mut app, side) in [(server_app(), "server"), (client_app(), "client")] {
            let names: Vec<String> = survey(&mut app).into_iter().filter(|s| s.sim).map(|s| s.name).collect();
            assert!(names.iter().any(|n| n.ends_with("sim_move")), "{side} doesn't run sim_move: {names:?}");
        }
    }

    // The checker itself: the bugs it exists to catch.

    fn predicts_from_hp(mut q: Query<(&mut Pose, &ActionState<Inputs>, &Hp)>) {
        for (mut pose, input, hp) in &mut q {
            if hp.0 > 0.0 {
                step(&mut pose, &input.0);
            }
        }
    }

    fn knockback(mut q: Query<&mut Pose>) {
        for mut pose in &mut q {
            pose.pos.x += 1.0;
        }
    }

    fn frozen_by_round(round: Res<RoundState>, mut q: Query<(&mut Pose, &ActionState<Inputs>)>) {
        if round.respawn_in.is_none() {
            for (mut pose, input) in &mut q {
                step(&mut pose, &input.0);
            }
        }
    }

    #[test]
    fn catches_known_mispredictions() {
        let mut app = client_app();
        app.init_resource::<RoundState>().add_systems(
            FixedUpdate,
            (predicts_from_hp.in_set(SimSet), frozen_by_round.in_set(SimSet), knockback),
        );
        app.add_systems(Update, sim_move.in_set(SimSet)).add_systems(Update, server_round.in_set(AuthoritySet));
        let v = violations(&mut app, false);
        let hit = |needle: &str| v.iter().any(|m| m.contains(needle));
        assert!(hit("predicts_from_hp") && hit("Hp"), "{v:#?}");
        assert!(hit("frozen_by_round") && hit("RoundState"), "{v:#?}");
        assert!(hit("knockback") && hit("outside SimSet"), "{v:#?}");
        assert!(hit("not FixedUpdate"), "{v:#?}");
        assert!(hit("server_round") && hit("server-only"), "{v:#?}");
    }
}
