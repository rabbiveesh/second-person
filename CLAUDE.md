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
   - `scripts/shots <out dir> 1280x720 390x844:touch 844x390:touch` screenshots the staged juice moments
     (`examples/juice_shots`) at any sizes (logical px; `:touch` = touch UI, fires by tapping). Check mobile with it.
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
| virtual_joystick 2.8 | on-screen touch stick (bevy_ui) |
| vleue_navigator 0.16 (`avian3d`) | navmesh pathfinding (polyanya), built from avian colliders, WASM-safe |

Dev builds: `dev` feature = `bevy/dynamic_linking` + `inspector`; mold via `.cargo/config.toml`; deps at opt-level 3.
Machine-local cargo settings (e.g. `rustc-wrapper = "kache"`, a shared build cache across worktrees) go in the
gitignored `.cargo/config.local.toml`, which `.cargo/config.toml` includes if present.
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
  Engaged means he runs to cover (`arena::find_cover`, no shooting while running), preferring cover he reaches by
  running *across* your line of fire (`RADIAL_RUN_PENALTY`) and zig-zagging any leg that runs along it (`nav::Evade`,
  `nav::weave`), because magnetised shots (`combat::magnetised`) barely miss a man running straight down the line. then fights from it: hide, then
  strafe out to a peek spot (random side, random length, sometimes a quick glance), then duck back.
  After 1-3 peeks, or when hit, he `Relocate`s to a *different* cover (`arena::find_cover_avoiding`).
  Grappling hook (`combat::grapple`, `Grapple` on the target): while fighting and seeing you at 6-20m, he reels
  you in (shooter `Stunned { pull_to }`), fires a point-blank burst, leaves you stunned, then `Relocate`s.
  Investigating a noise he can't see (cover in the way) makes him walk toward it until he can.
  Dodge roll (`combat::dodge`, `Roll`): when he sees you fire straight at him from beyond `ROLL_MIN_RANGE`, he dives
  `ROLL_DISTANCE` across the line of fire (never into cover), then `ROLL_COOLDOWN` before the next, so a second shot
  lands. No return fire mid-roll. `juice` plays it as a shoulder roll: the view turns into the dive, somersaults forward a full turn while dropping `ROLL_DIP`, then squares back up.
  `Suspicion.last_known` is where he last saw or heard you (no omniscience).
  Tasks only set intent (`LookGoal`, `MoveTo`, `Activity`). Systems `perceive` → `gaze` → `walk`
  do the work. Use the shared `ARRIVE` constant for every arrival check, because mismatched thresholds deadlock tasks. Lower branches **fail** when a higher-priority condition appears (that's preemption).
  `Suspicion` fills while the shooter is in the view cone with line of sight; at 1.0 he's engaged.
  While engaged, any sighting tops it back up to 1.0 (contact); out of sight it drains, and at 0 he lets go.
  Peek spots keep `PEEK_SLACK` clear of every corner and stay inside his `VIEW_RANGE`, or peeks see nothing and the fight stalls.
  The engine supports a moving target: the camera is parented to him, and walking is just velocity on a kinematic body.
- `nav.rs`: vleue_navigator navmesh, built synchronously from `NavObstacle` colliders (the arena blocks) inside the
  layout's outline (`fit_navmesh` updates it each round).
  `MoveTo { dest, speed, strafe }` is planned into a `Route` and followed by `target::walk`. With `strafe` he moves
  without turning, so he can watch a threat while side-stepping.
- `Layout` also has the pure geometry helpers `is_clear`, `los_blocked` (top-down, cover and walls; all cover is taller
  than eyes), `find_cover` and `random_point`. They're unit-testable without an app; read the layout via `Res<Layout>`.
- `shooter.rs`: random start via `random_start` (clear of cover, ≥12m from the target).
- `shooter.rs`: dynamic capsule, rotation locked, tank controls relative to its own facing. One analog
  `ShooterAction::Drive` dual axis (x turn, y throttle); arrows bind to it as a virtual d-pad.
  Walking into the world emits `Bump` and a `Stagger` knockback (can't walk, only turn); his laser staggers you too.
  `Sidestep`: a double-tap straight left/right on `Drive` (arrows, or a double flick of the stick, since both feed the same
  axis) hops 2.4m sideways with the facing restored to before the taps; the landing writes a `Footstep`. Stagger cuts it.
- `touch.rs`: phone controls, spawned on the first touch. A floating `virtual_joystick` stick (left half,
  snapped to 8 arrow-key directions by `snap_8way`) and tap-right-to-fire write the shooter's `ActionState` in
  leafwing's `ManualControl` set. The HUD swaps the R/M/Tab hints for egui buttons once touch is on, plus a big
  whistle button above the radar (`TouchWhistle` keeps its rect so taps on it don't fire).
- `start.rs` (presentation): "tap to play" overlay. `Time<Virtual>` is paused and the shooter's actions disabled
  until the first touch/key/click (`Started`). Then `TouchControls` follows the last input (touch on, key/click
  off), and the stick spawns/despawns with it. `scripts/headless-run` starts on this overlay: send any key over BRP.
- `arena.rs` floor zones: the `Floors` resource (base `Floor` + rect `FloorZone`s, later ones win; `Floors::at`).
  Classic = wood plaza, gravel, metal, grass; random layouts get a random base and up to 3 zones that fit (`Floors::random`). Each floor has its own
  footstep sounds and a `hearing_range`; `combat::hear_movement` turns nearby steps, bumps and whistles
  (W, the shooter's "where am I?" sound, heard by him from 28m) into suspicion.
- `combat.rs`: bullets (CCD, collision events; `magnetised` bends a shot within `MAGNET_CONE`/`MAGNET_RANGE` of him with
  a clear line straight at him, the only aim assist: no lock-on, nothing that leaks where he is), hearing (shots and near misses raise `Alert` +
  suspicion), target hitscan return fire while engaged (`Aim`: out of sight he holds the angle where you vanished, so re-peeking
  the same corner within `HOLD_ANGLE_SECS` draws a shot after `REACQUIRE_SECS`), warning shots (`WarningShot`, deliberate misses near
  `last_known`) while suspicious but not engaged, win/lose check.
- `layout.rs` (presentation): `ScreenLayout` (view, deck, radar rects) refit from the window every frame, so
  rotation and resizes just work. Landscape: full-window view, radar in the corner over it. Portrait (h ≥ 1.3w): a
  deck below the view holds the radar and whistle, and the left thumb drives from it. FOV is Hor+ with a floor:
  `BASE_VFOV` vertical until the view shows less than `MIN_HFOV` across, then the vertical opens up to `MAX_VFOV`.
  egui draws after every camera, so HUD fills must leave holes for camera viewports (see `hud::fill_around`).
- `radar.rs`: ortho top-down camera in the viewport `layout` gives it (layers 0+1), framed on the layout's bounds. `RadarMode`
  is the player's pick (Tab cycles; init'd in `round` so it exists headless):
  - Auto (default): sonar at the pace the assist dial sets (see `difficulty.rs`), none once the help has faded.
  - Full: live `LiveBlip`s, heading arrow and view cone.
  - Sonar: a sweep every 2s spawns fading contacts at each `RadarContact`, and gunshots ping too.
  - Off: no radar.
  Shooter and bullets render on layer 0 only, so the radar can't see them except through blips and contacts.
  The radar is lit by a per-camera `AmbientLight`, not a light of its own. Any shadow-casting light on the radar
  layer leaks the actors' shadows onto it, and **WebGL2 allows one `DirectionalLight` in total** (the sun). Tests guard both.
  bevy_firework is vendored (`vendor/bevy_firework`, `[patch.crates-io]`) with a fix so particles work under MSAA on
  WebGL2 (it bound a multisampled dummy depth texture, which WebGL2 can't create). Drop the patch once upstream fixes it.
- `adapt/` (pure, no Bevy): the adaptive difficulty engine. A reducer (`reduce(PlayerProfile, AdaptEvent)`) keeps a
  band 1..10, spread and rolling window per `Skill` (Stealth: landing the first hit before he engages; Gunfight: winning
  once he has), plus an assist dial 0..1 kept separate from the bands. Each round's bands are sampled around the
  centers (band blending), so promotion is never a cliff. Assists fade before a band rises; losing streaks ease off.
  The first rounds are a disguised placement test. `adapt::sim` plays synthetic players through the real reducer
  (`cargo run --example simulate -- --all --seeds 20`); tuning notes in `src/adapt/README.md`.
- `difficulty.rs`: what the engine's numbers do in play. `Tuning` (Reflect, queryable over BRP) is this round's bands +
  dial; `HisLevers` (from the bands: view cone, how fast he spots you, hearing, fire rate/damage, holding angles,
  grapple/roll, HP) and `AssistLevers` (from the dial: magnetism, sonar pace, damage taken). Band 5 and the baseline dial
  are exactly the hand-tuned constants, which stay in `target`/`combat`/`radar` (a test pins this). `AdaptiveDifficulty`
  is only inserted by `main.rs`, so headless tests and staged examples keep the fixed tuning. Never shown to the player.
  The 1v1 duel will have no assists.
- `fx.rs`: bevy_firework particle bursts (muzzle, impacts, hits), a muzzle point light (lights up the area
  around the shooter even when he's off-screen), and return-fire tracers.
- `juice.rs`: transform-only feel, so it runs headless and is tested: the eyes flinch along the bullet
  (spring + shake on `CameraJuice`, layered on the `MainCamera`, which the AI never moves), drop and roll to the
  floor when he dies (`OnEnter(Won)`), and the shooter topples when killed (`OnEnter(Lost)`). Part of `presentation`.
- `audio.rs`: bevy_kira_audio with our own spatial mix (`audio::mix`, not kira's spatial plugin): pan capped
  at ±0.3 (one-earbud friendly), inverse-distance falloff, and a low-passed `_muffled` twin crossfaded in for
  sounds behind the listener or behind cover. The listener is the target's head (`MainCamera`). SFX come from
  `cargo run --example gen_sfx` (sfxr, fixed seeds) and are written to `assets/sfx/`.
- Combat emits messages (`Gunshot`, `BulletImpact`, `TargetHit`, `ShooterHit`, shooter `Footstep`); fx, audio
  and radar subscribe to them. Add new feedback by subscribing, not by calling across modules.
- `hud.rs`: egui on a dedicated `Camera2d` overlay (`PrimaryEguiContext`; auto-context disabled
  because game cameras respawn each round). Bars, activity, flashes, hit marker,
  wound vignette, radar frame, end banner (held back `BANNER_DELAY` so the deaths play out).

## Tuning knobs
Per-band and per-dial curves in `difficulty.rs` (the hand-tuned values below are band 5 / the baseline dial); the
engine's thresholds in `adapt/profile.rs`, checked with the simulator. Suspicion rates in `target::perceive`. `VIEW_HALF_ANGLE`/`VIEW_RANGE` in `target.rs`. Return fire
damage/interval, holding the angle (`HOLD_ANGLE_*`, `REACQUIRE_SECS`), bullet magnetism (`MAGNET_CONE`, `MAGNET_RANGE`), hearing range and near-miss range in `combat.rs`.
Field of view and the portrait threshold in `layout.rs`.
