# Second Person Shooter

You see the world through the **target's** eyes (the main camera lives in his head) while
controlling the **shooter** hunting him. A corner radar shows the whole arena.

## Ground rules
- **Use frameworks at every opportunity.** Don't hand-roll what a maintained crate does.
- **Rust everywhere.**
- Versions are pinned to **Bevy 0.19.1**. Model knowledge of Bevy APIs is often stale, so before
  using an API, grep the crate source/examples in `~/.cargo/registry/src/*/<crate>-<ver>/`.
  Read the crate's RELEASES/CHANGELOG when a pattern doesn't compile.
- **Never use xdotool or anything that grabs the user's screen/focus/keyboard.** The user works on this
  machine. Avoid long CPU-heavy builds (e.g. `trunk build --release`) without asking; CI does those.

## Dev loop
1. `cargo build` with no warnings.
2. `cargo test`: headless gameplay tests in `tests/gameplay.rs`. They run the real `gameplay` plugins on
   `MinimalPlugins` with a fixed 60 Hz clock. Call `app.finish(); app.cleanup();` before
   `update()` (avian inits resources in `finish`). Prefer adding a test over eyeballing.
   For AI bugs, trace state per frame in a test (see `TRACE=1 cargo test ... -- --nocapture` in
   `engaged_target_eventually...`) rather than guessing.
3. To see or drive the real game: `scripts/headless-run`. This uses Xvfb with software Vulkan (lavapipe), so no window
   appears. Then `scripts/brp` talks to that instance's BRP on **:15799** (feature `brp`, on in `dev`).
   The user's own `cargo run` uses :15702. **Never send BRP to 15702** unless the user asks, because that's their live game.
   - `scripts/brp world.query '{"data":{"components":["second_person::target::Suspicion"]},"filter":{}}'`
   - `scripts/brp brp_extras/send_keys '{"keys":["Space"],"duration_ms":60}'`
   - `scripts/brp brp_extras/screenshot '{"path":"<scratchpad>/shot.png"}'`
   Gameplay components derive `Reflect` + `#[reflect(Component)]` (auto-registered) so they're queryable.
   `.mcp.json` also registers `bevy_brp_mcp` for MCP-native access.

## Stack (and why)
| Crate | Role |
|---|---|
| bevy 0.19 | engine / ECS / rendering / multi-camera viewports |
| avian3d 0.7 | physics, collision events, raycasts (line of sight) |
| leafwing-input-manager 0.21 | input → actions. 0.21 has **no resource ActionState**; put global inputs on an entity |
| bevior_tree 0.11 (`default-features = false`, serde feature needs typetag impls) | target AI behaviour tree. Picked over big-brain, which is stuck on Bevy 0.15; vendor big-brain if bevior_tree hurts |
| bevy_egui 0.40 + bevy-inspector-egui 0.37 | HUD + F1 world inspector (inspector pins egui 0.40; `inspector` feature) |
| bevy_firework 0.10 | CPU particles (WebGL-safe; bevy_hanabi needs compute, i.e. WebGPU only) |
| bevy_kira_audio 0.26 (`wav`) | spatial audio. Bevy built with `default-features = false, features = ["3d","ui"]` to drop bevy_audio |
| sfxr + hound (dev-deps) | procedural SFX generation |
| bevy_brp_extras 0.22 | BRP + screenshots/key input for agents (`brp` feature) |
| rand 0.9 | randomness |
| earcut 0.4 | triangulates arena floor polygons (already in the tree via vleue_navigator) |
| vleue_navigator 0.16 (`avian3d`) | navmesh pathfinding (polyanya), built from avian colliders, WASM-safe |

Dev builds: `dev` feature = `bevy/dynamic_linking` + `inspector`; mold via `.cargo/config.toml`; deps at opt-level 3.
Web/Pages: `trunk build --release` uses the `wasm-release` profile, which uses **fat LTO on purpose**.
Deploys are slower (~6 min warm vs ~3 for thin), but the download is ~3MB smaller, and the dev loop never uses that profile.
System deps (Ubuntu): `libudev-dev`, `libasound2-dev`, `libwayland-dev`, `libxkbcommon-dev`.

Considered alternatives: Godot+gdext (editor, but FFI borrow friction and scenes in .tscn),
Fyrox (small ecosystem), macroquad/three-d (too thin, would mean rolling our own).

## Shape (`src/`)
- `lib.rs`: `gameplay` (headless simulation) vs `presentation` (HUD/radar/fx/audio) plugin groups; `Layer` physics layers.
- `main.rs`: window, egui, and feature-gated dev tools (`inspector`: F1; `brp`).
- `round.rs`: `GameState` (Playing/Won/Lost). Per-round entities get `RoundEntity` and are
  despawned on `OnEnter(Playing)` before the `SpawnRound` set. Physics pauses outside Playing.
  `MetaAction` (R restart, M toggle target walking). `TargetMobile` resource.
- `arena.rs`: `Layout` resource (floor outline polygon + axis-aligned cover `Block`s), respawned every round
  (`RoundEntity`). `ArenaMode` picks it: `Classic` (the original hand-placed arena), `Random` (default; new
  `Layout::random(seed)` each round: yard, hall, L, cross, octagon or notch, rotated, with scattered cover), or
  `Seed(n)`. L cycles Classic/Random and restarts. Generated cover keeps `COVER_GAP` from walls and other cover so the
  free space stays connected, and keeps the origin (the target's spawn) open. The floor mesh is triangulated with `earcut`.
- `target.rs`: target entity → Head (pitch) → `MainCamera`. Behaviour tree:
  `Selector[engaged→(TakeCover, Fight), alerted→Investigate, mobile→Wander, Scan]`.
  Engaged means he runs to cover (`arena::find_cover`, no shooting while running), then fights from it: hide, then
  strafe out to a peek spot (shooting if he sees you), then duck back. Getting hit makes him re-plan cover.
  `Suspicion.last_known` is where he last saw or heard you (no omniscience).
  Tasks only set intent (`LookGoal`, `MoveTo`, `Activity`). Systems `perceive` → `gaze` → `walk`
  do the work. Use the shared `ARRIVE` constant for every arrival check, because mismatched thresholds deadlock tasks. Lower branches **fail** when a higher-priority condition appears (that's preemption).
  `Suspicion` fills while the shooter is in the view cone with line of sight; at 1.0 he's engaged.
  The engine supports a moving target: the camera is parented to him, and walking is just velocity on a kinematic body.
- `nav.rs`: vleue_navigator navmesh, built synchronously from `NavObstacle` colliders (the arena blocks) inside the
  layout's outline (`fit_navmesh` updates it each round).
  `MoveTo { dest, speed, strafe }` is planned into a `Route` and followed by `target::walk`. With `strafe` he moves
  without turning, so he can watch a threat while side-stepping.
- `Layout` also has the pure geometry helpers `is_clear`, `los_blocked` (top-down, cover and walls; all cover is taller
  than eyes), `find_cover` and `random_point`. They're unit-testable without an app; read the layout via `Res<Layout>`.
- `shooter.rs`: random start via `random_start` (clear of cover, ≥12m from the target).
- `shooter.rs`: dynamic capsule, rotation locked, tank controls (arrows), relative to its own facing.
- `combat.rs`: bullets (CCD, collision events), hearing (shots and near misses raise `Alert` +
  suspicion), target hitscan return fire while engaged, win/lose check.
- `radar.rs`: ortho top-down camera in a bottom-right viewport (layers 0+1), reframed to the layout's bounds. `RadarMode` is the difficulty knob
  (Tab cycles; init'd in `round` so it exists headless):
  - Full: live `LiveBlip`s, heading arrow and view cone.
  - Sonar (default): a sweep every 2s spawns fading contacts at each `RadarContact`, and gunshots ping too.
  - Off: no radar.
  Shooter and bullets render on layer 0 only, so the radar can't see them except through blips and contacts.
  The radar is lit by a per-camera `AmbientLight`, not a light of its own. Any shadow-casting light on the radar
  layer leaks the actors' shadows onto it, and **WebGL2 allows one `DirectionalLight` in total** (the sun). Tests guard both.
  bevy_firework is vendored (`vendor/bevy_firework`, `[patch.crates-io]`) with a fix so particles work under MSAA on
  WebGL2 (it bound a multisampled dummy depth texture, which WebGL2 can't create). Drop the patch once upstream fixes it.
- `fx.rs`: bevy_firework particle bursts (muzzle, impacts, hits), a muzzle point light (lights up the area
  around the shooter even when he's off-screen), and return-fire tracers.
- `audio.rs`: bevy_kira_audio spatial one-shots. The listener is the target's head (`MainCamera`). SFX come from
  `cargo run --example gen_sfx` (sfxr, fixed seeds) and are written to `assets/sfx/`.
- Combat emits messages (`Gunshot`, `BulletImpact`, `TargetHit`, `ShooterHit`, shooter `Footstep`); fx, audio
  and radar subscribe to them. Add new feedback by subscribing, not by calling across modules.
- `hud.rs`: egui on a dedicated `Camera2d` overlay (`PrimaryEguiContext`; auto-context disabled
  because game cameras respawn each round). Bars, activity, flashes, radar frame, end banner.

## Tuning knobs
Suspicion rates in `target::perceive`. `VIEW_HALF_ANGLE`/`VIEW_RANGE` in `target.rs`. Return fire
damage/interval, hearing range and near-miss range in `combat.rs`.
