// Headless test harness: the real Game on Babylon's NullEngine (no GPU, no canvas), with a
// fixed 60 Hz clock so runs are deterministic.

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import HavokPhysics from "@babylonjs/havok";
import { Logger, NullEngine, Vector3 } from "@babylonjs/core";
import { FIXED_DT, Game } from "../src/game";
import type { Bindings } from "../src/input";

Logger.LogLevels = Logger.WarningLogLevel | Logger.ErrorLogLevel;

const require = createRequire(import.meta.url);
let havok: Promise<unknown> | undefined;

/** Small seeded PRNG (mulberry32) so AI choices are reproducible. */
export function seeded(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export async function makeGame(seed = 1): Promise<Game> {
  havok ??= HavokPhysics({
    wasmBinary: readFileSync(require.resolve("@babylonjs/havok/lib/esm/HavokPhysics.wasm")).buffer as ArrayBuffer,
  });
  const engine = new NullEngine({
    deterministicLockstep: true,
    lockstepMaxSteps: 1,
    renderWidth: 1280,
    renderHeight: 720,
    textureSize: 256,
  } as never);
  // Every render() is exactly one fixed step.
  engine.getDeltaTime = () => FIXED_DT * 1000;
  const game = await Game.create(engine, await havok, seeded(seed));
  tick(game);
  return game;
}

export function tick(game: Game, n = 1) {
  for (let i = 0; i < n; i++) game.scene.render();
}

export function run(game: Game, secs: number) {
  tick(game, Math.ceil(secs / FIXED_DT));
}

export function press(game: Game, key: (typeof Bindings)[keyof typeof Bindings]) {
  game.input.press(key);
  tick(game);
  game.input.release(key);
  tick(game);
}

/** Yaw that makes `from` face `to` (forward is -Z). */
export function yawTowards(from: Vector3, to: Vector3): number {
  const d = to.subtract(from);
  return Math.atan2(-d.x, -d.z);
}

/** Put the target at the origin facing `point`, and the shooter at `shooterPos` facing the target. */
export function stage(game: Game, shooterPos: Vector3, targetFaces: Vector3) {
  const t = new Vector3(0, 0.9, 0);
  game.target.place(t, yawTowards(t, targetFaces));
  game.shooter.place(shooterPos, yawTowards(shooterPos, t));
  tick(game);
}
