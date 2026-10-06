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

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct Hp(f32);

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct Score(u32);

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
    app.component::<Pose>()
        .replicate()
        .predict()
        .add_linear_interpolation()
        .add_correction();
    app.component::<Hp>().replicate();
    app.component::<Score>().replicate();
    app.component::<BulletPos>().replicate().add_linear_interpolation();
    app.register_message::<NetEvent>()
        .add_direction(NetworkDirection::ServerToClient);
    app.add_channel::<Reliable>(ChannelSettings {
        mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
        ..default()
    })
    .add_direction(NetworkDirection::ServerToClient);
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
                MinimalPlugins.set(bevy::app::ScheduleRunnerPlugin::run_loop(Duration::from_millis(2))),
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
            (server_move, record_history, server_fire, server_bullets, server_round).chain(),
        )
        .add_systems(Update, server_log);
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
        Hp(MAX_HP),
        Score(0),
        Gun { cooldown: 0 },
        History::default(),
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::Single(id)),
        InterpolationTarget::to_clients(NetworkTarget::AllExceptSingle(id)),
        ControlledBy { owner: t.entity, lifetime: Default::default() },
    ));
}

fn server_move(round: Res<RoundState>, mut q: Query<(&mut Pose, &ActionState<Inputs>, &Hp)>) {
    if round.respawn_in.is_some() {
        return;
    }
    for (mut pose, input, hp) in &mut q {
        if hp.0 > 0.0 {
            step(&mut pose, &input.0);
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
    mut q: Query<(&PlayerId, &Pose, &ActionState<Inputs>, &mut Gun, &ControlledBy, &Hp)>,
    delays: Query<&InterpolationDelay, With<ClientOf>>,
) {
    for (id, pose, input, mut gun, controlled, hp) in &mut q {
        gun.cooldown = gun.cooldown.saturating_sub(1);
        if !input.0.fire || gun.cooldown > 0 || hp.0 <= 0.0 || round.respawn_in.is_some() {
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

fn server_round(mut round: ResMut<RoundState>, mut q: Query<(&mut Pose, &mut Hp)>) {
    let Some(n) = round.respawn_in.as_mut() else { return };
    *n -= 1;
    if *n > 0 {
        return;
    }
    round.respawn_in = None;
    let mut first = None;
    for (mut pose, mut hp) in &mut q {
        *pose = random_spawn(first);
        first = Some(pose.pos);
        hp.0 = MAX_HP;
    }
    info!("new round");
}

// ---------------------------------------------------------------- client

#[derive(Resource)]
struct BotBrain {
    enabled: bool,
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
    app.insert_resource(BotBrain { enabled: bot, aim_error: 0.0, retarget: 0 })
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
        .add_systems(FixedUpdate, predict_move)
        .add_systems(Update, (receive_events, log_state));
    if !headless {
        app.add_systems(Startup, setup_view)
            .add_plugins(arena::plugin)
            .add_systems(FixedUpdate, local_fire_fx.after(predict_move).run_if(not(is_in_rollback)))
            .add_systems(
                Update,
                (dress_players, dress_bullets, sync_transforms, eyes_follow_opponent, tick_fx, hud_title, shots),
            );
    }
}

/// Your inputs: arrows + space, or the bot. The bot "cheats" the way any client could:
/// it knows the opponent's position because it has to, to render their eyes.
fn write_inputs(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut me: Query<(&mut ActionState<Inputs>, &Pose), With<InputMarker<Inputs>>>,
    them: Query<&Pose, (With<Interpolated>, With<PlayerId>)>,
    mut brain: ResMut<BotBrain>,
) {
    let Ok((mut action, me)) = me.single_mut() else { return };
    let mut i = Inputs::default();
    if brain.enabled {
        if let Ok(them) = them.single() {
            let mut rng = rand::rng();
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
            i.fire = clear_shot && err.abs() < 0.15 && to.length() < 35.0;
        }
    } else if let Some(k) = keys {
        i.fwd = k.pressed(KeyCode::ArrowUp) as i8 - k.pressed(KeyCode::ArrowDown) as i8;
        i.turn = k.pressed(KeyCode::ArrowLeft) as i8 - k.pressed(KeyCode::ArrowRight) as i8;
        i.fire = k.pressed(KeyCode::Space);
    }
    action.0 = i;
}

fn predict_move(mut q: Query<(&mut Pose, &ActionState<Inputs>, &Hp), With<Predicted>>) {
    for (mut pose, input, hp) in &mut q {
        if hp.0 > 0.0 {
            step(&mut pose, &input.0);
        }
    }
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
    them: Query<&Pose, (With<Interpolated>, With<PlayerId>)>,
    mut eyes: Single<&mut Transform, With<Eyes>>,
) {
    if let Ok(p) = them.single() {
        eyes.translation = Vec3::new(p.pos.x, EYE_HEIGHT, p.pos.y) + dir(p.yaw).extend(0.0).xzy() * 0.36;
        eyes.rotation = Quat::from_rotation_y(p.yaw);
    }
}

/// Instant feedback for your own shots: a local cosmetic bullet from the predicted pose.
/// The server's real bullet is never sent back to you; hits come back as events.
fn local_fire_fx(
    mut commands: Commands,
    q: Query<(&Pose, &ActionState<Inputs>, &Hp), With<Predicted>>,
    mut cooldown: Local<u32>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    *cooldown = cooldown.saturating_sub(1);
    let Ok((pose, input, hp)) = q.single() else { return };
    if !input.0.fire || *cooldown > 0 || hp.0 <= 0.0 {
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
    me: Query<(&Hp, &Score), With<Predicted>>,
    them: Query<&Score, (With<Interpolated>, With<PlayerId>)>,
    mut window: Single<&mut Window>,
    mut clear: ResMut<ClearColor>,
) {
    let dt = time.delta_secs();
    hud.flash = (hud.flash - dt).max(0.0);
    hud.msg_t = (hud.msg_t - dt).max(0.0);
    let title = match (me.single(), them.single()) {
        (Ok((hp, s)), Ok(os)) => {
            hud.my_hp = hp.0;
            format!(
                "HP {:.0}   score {} - {}   {}",
                hp.0,
                s.0,
                os.0,
                if hud.msg_t > 0.0 { hud.msg.as_str() } else { "arrows move/turn, space fires; you see through THEIR eyes" }
            )
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
