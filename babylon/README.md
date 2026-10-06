# Second Person Shooter, in Babylon.js

The same game as the Bevy version at the repo root, rewritten in **Babylon.js 9 + TypeScript**
(Vite for the dev server and build) to get a feel for a different kind of game framework.
The Bevy game is untouched; this directory is self-contained apart from sharing `../assets/sfx`.

```sh
npm install
npm run dev       # http://localhost:5173, hot reload on save
npm test          # headless gameplay tests on Babylon's NullEngine (~3s)
npm run build     # static site in dist/ (~5s)
```

Controls are the same: arrows to drive, Space to fire, Tab cycles the radar, M lets the target
walk, R restarts after a round ends.

## What maps to what

| Bevy version | Babylon version |
|---|---|
| ECS: entities + components + systems, scheduled by Bevy | Scene graph of nodes/meshes, plus plain classes (`Shooter`, `Target`, `Combat`) with a `step(dt)` the `Game` calls in order |
| `gameplay` / `presentation` plugin groups | `Game` (works on a `NullEngine`, no GPU) vs `Radar`, `Fx`, `Hud`, `startAudio` subscribing to it |
| `Message`s (`Gunshot`, `TargetHit`, …) | `Observable`s on `GameEvents` |
| avian3d (dynamic capsule, CCD bullets, raycasts) | Havok (wasm, bundled with Babylon): character controller for the shooter, kinematic body for the target, swept raycasts for bullets |
| vleue_navigator (2D polyanya from collider outlines) | Recast/Detour via `@babylonjs/addons` navigation, voxelised from the arena meshes |
| bevior_tree, tasks written as systems | mistreevous, tree written in its text DSL; branches preempt with `while(...)` guards |
| leafwing-input-manager | ~60 lines of `input.ts` over `scene.onKeyboardObservable` |
| bevy_firework (vendored for a WebGL2 fix) | built-in `ParticleSystem` |
| bevy_kira_audio | built-in AudioV2 (Web Audio), listener attached to the target's camera |
| bevy_egui (immediate mode) | `@babylonjs/gui` (retained-mode control tree) |
| `RenderLayers` + per-camera `AmbientLight` | `layerMask` on meshes/cameras; the radar draws its own flat map |
| `States` + `RoundEntity` despawn | a `Round` object whose root node is disposed on restart (children and physics bodies go with it) |
| Fixed 60 Hz clock in tests | Engine `deterministicLockstep`; gameplay runs in `scene.onBeforeStepObservable` |

## Trade-offs I ran into

Written while porting, roughly in order of how much they mattered.

### Where Babylon was better

- **Iteration speed.** `vite build` takes ~5s and `npm test` ~3s cold. Dev mode hot-reloads on
  save. The Bevy loop needs dynamic linking, mold and opt-level tricks to be tolerable, and a
  web release build is ~6 minutes. This is the biggest day-to-day difference.
- **Batteries included, and they agree with each other.** Physics, navmesh, particles, spatial
  audio, GUI, shadows and multi-camera viewports all ship from one vendor on one version. In
  Bevy each is a separate crate that has to have caught up with the same Bevy release (that's
  why big-brain was out and bevy_firework had to be vendored).
- **Download size.** The whole site is ~2.6 MB gzipped (JS + Havok wasm), versus ~12.3 MB for
  the Bevy wasm build with fat LTO. And that's before trimming: it imports Babylon's root barrel,
  which pulls in the entire engine (7.5 MB of JS, 1.6 MB gzipped). Deep imports would cut it.
- **The web is the native platform.** No wasm-bindgen, Trunk, getrandom feature flags, or
  "WebGL2 can't create a multisampled depth texture" surprises. Devtools work, `window.game`
  is inspectable from the console, and Playwright can drive it.
- **A character controller out of the box.** Havok's `PhysicsCharacterController` slides along
  walls with no rotation locks or friction-combine tweaks.
- **Declarative preemption in the behaviour tree.** mistreevous guards (`while(IsCalm)`) abort a
  running branch automatically. The Bevy tree needed every low-priority task to check
  `interrupted()` itself and fail.

### Where Bevy was better

- **Types and data-driven queries.** Everything here is a hand-wired object reference
  (`game.target.suspicion`). There's no "query every `RadarContact`" or `Changed<MoveTo>`: you
  keep your own lists (`liveBlips`) and call things explicitly. That's fine at this size, but
  the decoupling ECS gives for free has to be designed by hand as the game grows.
- **Ownership and lifetimes.** Forgetting to dispose something in Babylon leaks silently; Rust
  and ECS despawn make that much harder. The `Round` root node helps, but the character
  controller isn't a node and has to be disposed by hand.
- **Defaults that fail safe.** Babylon meshes default to `layerMask = 0x0FFFFFFF`, visible to
  every camera, so every world mesh has to opt out of the radar (the invisible target capsule
  forgot, and the ported "radar can't see actors" test caught it). Bevy's default is layer 0 only.
  Likewise the character controller's hidden body collided with and blocked rays from
  everything until it got its own shape with a filter. The line-of-sight test caught that one.
- **Compile-time checking of engine API use.** TypeScript catches a lot, but Babylon's
  `getPhysicsEngine()` is typed for both physics v1 and v2, and several options are bags of
  optional fields where a typo is silently ignored.
- **Immediate-mode HUD.** egui lays itself out. Babylon GUI is a retained tree you build once and
  mutate every frame, and containers don't grow to fit text (the end banner clipped until it got
  a hand-set width). A DOM overlay would be the other web-native option.
- **Per-camera lighting.** Bevy let the radar camera have its own ambient light over the real
  arena. Babylon lights are per-scene, so the radar draws a separate flat map instead.

### Surprises either way

- **Handedness.** Babylon is left-handed by default. Setting `scene.useRightHandedSystem` made
  all the Bevy maths (yaw, forward = -Z) port unchanged, but cameras then default to a half
  turn and look +Z, so the eye camera needs `rotation = 0` explicitly.
- **Lazy CDN loading.** The navigation addon fetches Recast from unpkg at runtime unless you
  inject the npm copy (`nav.ts` does). Good for a playground, surprising for a shipped game.
- **CCD.** Babylon's Havok plugin doesn't expose per-body continuous collision, so bullets are
  swept raycasts. That's arguably better anyway (cheaper, and can't tunnel).
- **Fixed timestep.** Babylon's `deterministicLockstep` gives the same "gameplay at 60 Hz,
  render at whatever" split Bevy's fixed schedules do. Tests swap in a `NullEngine` and
  stub `getDeltaTime`, so every `scene.render()` is exactly one step.

## Tests

`tests/gameplay.test.ts` ports `tests/gameplay.rs` (vision, cover, hearing, cover-seeking,
the 90-second "he eventually kills an exposed shooter" run, restart, navmesh routing, random
starts, radar layering) and adds driving/wall tests. `scripts/smoke.mjs` loads the built game in
headless Chromium with software WebGL, plays a few seconds and saves screenshots:

```sh
npm run build && npx vite preview --port 4173 &
CHROMIUM=/path/to/chrome node scripts/smoke.mjs /tmp
```
