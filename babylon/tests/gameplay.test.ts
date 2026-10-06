// Ports of the Bevy game's tests/gameplay.rs, against the real Game on a NullEngine.

import { Vector2, Vector3 } from "@babylonjs/core";
import { describe, expect, it } from "vitest";
import { findCover, isClear, losBlocked, xz } from "../src/arena";
import { MIN_START_DISTANCE, SHOOTER_MAX_HP, randomStart } from "../src/shooter";
import { TARGET_MAX_HP } from "../src/target";
import { makeGame, press, run, seeded, stage, tick } from "./harness";

const count = (o: { add(f: () => void): unknown }) => {
  const c = { n: 0 };
  o.add(() => c.n++);
  return c;
};

describe("gameplay", () => {
  it("round starts with one of each", async () => {
    const game = await makeGame();
    expect(game.state).toBe("Playing");
    expect(game.shooter.hp).toBe(SHOOTER_MAX_HP);
    expect(game.target.hp).toBe(TARGET_MAX_HP);
    expect(game.scene.activeCamera).toBe(game.target.camera);
  });

  it("firing spawns a bullet and a gunshot", async () => {
    const game = await makeGame();
    const shots = count(game.events.gunshot);
    game.input.press("Space");
    tick(game);
    expect(shots.n).toBe(1);
    expect(game.round.combat.bullets.length).toBe(1);
  });

  it("the shooter drives forward and turns", async () => {
    const game = await makeGame();
    const start = new Vector3(0, 0.9, 12);
    game.shooter.place(start, 0); // facing -Z, toward the origin
    tick(game);
    game.input.press("ArrowUp");
    run(game, 0.5);
    game.input.release("ArrowUp");
    const moved = game.shooter.position.subtract(start);
    expect(moved.z).toBeLessThan(-1.5);
    expect(Math.abs(moved.x)).toBeLessThan(0.1);
    expect(Math.abs(game.shooter.position.y - 0.9)).toBeLessThan(0.2);

    game.input.press("ArrowLeft");
    run(game, 0.5);
    expect(game.shooter.yaw).toBeGreaterThan(1);
  });

  it("walls stop the shooter", async () => {
    const game = await makeGame();
    game.shooter.place(new Vector3(0, 0.9, 25), Math.PI); // facing +Z, toward the wall at z=30
    tick(game);
    game.input.press("ArrowUp");
    run(game, 3);
    expect(game.shooter.position.z).toBeLessThan(29.5 - 0.3);
  });

  it("shooting the target hurts and alerts him", async () => {
    const game = await makeGame();
    const hits = count(game.events.targetHit);
    // Shooter right behind him: point blank, and out of his view.
    stage(game, new Vector3(0, 0.9, 6), new Vector3(0, 0.9, -10));
    press(game, "Space");
    run(game, 0.5);
    expect(hits.n).toBe(1);
    expect(game.target.hp).toBe(TARGET_MAX_HP - 1);
    expect(game.target.suspicion.level).toBeGreaterThan(0.5);
  });

  it("he sees the shooter in his view", async () => {
    const game = await makeGame();
    const shooterPos = new Vector3(0, 0.9, -10);
    stage(game, shooterPos, shooterPos);
    tick(game);
    expect(game.target.suspicion.seesShooter).toBe(true);
    expect(game.target.suspicion.level).toBeGreaterThan(0);
  });

  it("he doesn't see the shooter behind him", async () => {
    const game = await makeGame();
    stage(game, new Vector3(0, 0.9, 10), new Vector3(0, 0.9, -10));
    tick(game);
    expect(game.target.suspicion.seesShooter).toBe(false);
  });

  it("cover blocks line of sight", async () => {
    const game = await makeGame();
    // Crate at (-6, _, -8); shooter directly behind it as seen from the origin.
    const shooterPos = new Vector3(-9, 0.9, -12);
    stage(game, shooterPos, shooterPos);
    tick(game);
    expect(game.target.suspicion.seesShooter).toBe(false);
  });

  it("nearby gunshot makes him investigate", async () => {
    const game = await makeGame();
    // Behind him (out of view) but within hearing range, firing away from him.
    stage(game, new Vector3(0, 0.9, 10), new Vector3(0, 0.9, -10));
    game.shooter.place(new Vector3(0, 0.9, 10), Math.PI);
    press(game, "Space");
    run(game, 0.2);
    expect(game.target.alert).toBeDefined();
    expect(game.target.activity).toBe("Investigating");
  });

  it("engaged target runs for cover", async () => {
    const game = await makeGame();
    const shooterPos = new Vector3(0, 0.9, -12);
    stage(game, shooterPos, shooterPos);
    game.target.suspicion.bump(1);
    game.target.suspicion.lastKnown = shooterPos;
    let tookCover = false;
    let hidden = false;
    for (let i = 0; i < 6 * 60; i++) {
      tick(game);
      tookCover ||= game.target.activity === "TakingCover";
      hidden ||= losBlocked(xz(shooterPos), xz(game.target.position));
    }
    expect(tookCover, "never ran for cover").toBe(true);
    expect(hidden, "never got out of the shooter's line of sight").toBe(true);
  });

  it("engaged target eventually kills an exposed shooter, then R restarts", async () => {
    const game = await makeGame();
    const hurt = count(game.events.shooterHit);
    const shooterPos = new Vector3(0, 0.9, -8);
    stage(game, shooterPos, shooterPos);
    game.target.suspicion.bump(1);

    // He hides and peeks, so this takes a while; a shooter standing in the open still loses.
    for (let i = 0; i < 90 * 60 && game.state === "Playing"; i++) {
      tick(game);
      if (process.env.TRACE && i % 30 === 0) {
        const t = game.target;
        const s = t.suspicion;
        console.log(
          `t=${(i / 60).toFixed(1)} ${t.activity} pos=${xz(t.position)} lvl=${s.level.toFixed(2)} ` +
            `eng=${s.engaged} sees=${s.seesShooter} route=${t.route.map((p) => xz(p).toString()).join(" ")}`,
        );
      }
    }
    expect(hurt.n).toBeGreaterThan(0);
    expect(game.state).toBe("Lost");

    const oldCamera = game.target.camera;
    press(game, "KeyR");
    expect(game.state).toBe("Playing");
    expect(game.shooter.hp).toBe(SHOOTER_MAX_HP);
    expect(oldCamera.isDisposed()).toBe(true);
    expect(game.scene.cameras).toEqual([game.target.camera]);
  });

  it("navmesh routes around cover", async () => {
    const game = await makeGame();
    // Straight line from (-10,-6) to (-10,2) goes through the pillar at (-10,-2).
    const a = new Vector3(-10, 0.9, -6);
    const b = new Vector3(-10, 0.9, 2);
    const path = game.nav.path(a, b);
    expect(path.length).toBeGreaterThan(1);
    expect(Vector2.Distance(xz(path[path.length - 1]), xz(b))).toBeLessThan(0.5);
    let prev = xz(a);
    for (const w of path) {
      for (let i = 0; i <= 20; i++) {
        const p = Vector2.Lerp(prev, xz(w), i / 20);
        expect(isClear(p, 0.3), `path passes through cover at ${p}`).toBe(true);
      }
      prev = xz(w);
    }
  });

  it("the target walks around when mobile", async () => {
    const game = await makeGame(3);
    game.input.press("KeyM");
    tick(game);
    expect(game.targetMobile).toBe(true);
    game.shooter.place(new Vector3(27, 0.9, 27), 0); // tucked in a corner, out of the way
    const start = game.target.position.clone();
    run(game, 3);
    expect(game.target.activity).toBe("Wandering");
    expect(Vector3.Distance(start, game.target.position)).toBeGreaterThan(2);
  });

  it("radar can't see actors directly", async () => {
    const game = await makeGame();
    const radarBit = 0x2;
    const blips = new Set<unknown>([...game.shooter.liveBlips, ...game.target.liveBlips]);
    for (const m of game.scene.meshes) {
      if (blips.has(m)) continue;
      expect(m.layerMask & radarBit, `${m.name} is visible on radar`).toBe(0);
    }
  });
});

describe("arena geometry", () => {
  it("cover spots hide from the threat", () => {
    for (const threat of [new Vector2(0, -12), new Vector2(15, 15), new Vector2(-20, 0)]) {
      const cover = findCover(Vector2.Zero(), threat);
      expect(cover, "some cover exists").toBeDefined();
      expect(losBlocked(threat, cover!.spot)).toBe(true);
      expect(isClear(cover!.spot, 0.5)).toBe(true);
    }
  });

  it("random starts are clear of cover and the target", () => {
    const rng = seeded(42);
    for (let i = 0; i < 500; i++) {
      const { pos } = randomStart(rng);
      const p = xz(pos);
      expect(p.length()).toBeGreaterThanOrEqual(MIN_START_DISTANCE);
      expect(isClear(p, 0.35)).toBe(true);
    }
  });
});
