// Visual effects driven by combat events: Babylon's built-in CPU particle system (bursts for
// muzzle, impacts and hits), a muzzle point light, return-fire tracers and shadows.
// The muzzle flash is a navigation aid: it lights up the shooter's surroundings even when
// he's off-screen.

import {
  Color3,
  Color4,
  DynamicTexture,
  type Mesh,
  MeshBuilder,
  ParticleSystem,
  PointLight,
  type Scene,
  ShadowGenerator,
  type Texture,
  Vector3,
} from "@babylonjs/core";
import type { Game } from "./game";
import { Mask } from "./layers";

interface Expiring {
  dispose(): void;
  ttl: number;
}

export class Fx {
  private expiring: Expiring[] = [];
  private spark: Texture;
  private shadows: ShadowGenerator;

  constructor(private game: Game) {
    const scene = game.scene;
    this.spark = sparkTexture(scene);

    this.shadows = new ShadowGenerator(2048, game.sun);
    this.shadows.usePercentageCloserFiltering = true;
    this.shadows.bias = 0.002;
    for (const m of game.arena.obstacles) this.shadows.addShadowCaster(m);
    game.events.roundStarted.add(() => this.onRound());
    this.onRound();

    const ev = game.events;
    ev.gunshot.add(({ muzzle, dir }) => {
      const light = new PointLight("muzzle light", muzzle, scene);
      light.diffuse = new Color3(1, 0.75, 0.4);
      light.intensity = 6;
      light.range = 14;
      light.includeOnlyWithLayerMask = Mask.World;
      this.expire(light, 0.08);
      this.burst(muzzle, dir, 0.35, 16, [6, 14], new Color4(1, 0.6, 0.2, 1), 0.18);
    });
    ev.bulletImpact.add(({ at }) => this.burst(at, Vector3.Up(), 1.2, 12, [2, 6], new Color4(1, 0.85, 0.5, 1), 0.35));
    ev.targetHit.add(({ at }) => this.burst(at, Vector3.Up(), 1.5, 20, [1, 4], new Color4(0.8, 0, 0, 1), 0.5));
    ev.shooterHit.add(({ from, to }) => {
      const line = MeshBuilder.CreateLines("tracer", { points: [from, to] }, scene);
      line.color = new Color3(1, 0.3, 0.2);
      line.layerMask = Mask.World;
      this.expire(line, 0.12);
    });

    scene.onBeforeRenderObservable.add(() => {
      const dt = scene.getEngine().getDeltaTime() / 1000;
      this.expiring = this.expiring.filter((e) => {
        e.ttl -= dt;
        if (e.ttl > 0) return true;
        e.dispose();
        return false;
      });
    });
  }

  private onRound() {
    const s = this.game.shooter;
    for (const m of [s.mesh, ...s.mesh.getChildMeshes()]) {
      if (m.layerMask === Mask.World) this.shadows.addShadowCaster(m as Mesh, false);
    }
  }

  private expire(thing: { dispose(): void }, secs: number) {
    this.expiring.push({ dispose: () => thing.dispose(), ttl: secs });
  }

  /** One-shot particle burst, disposed when done. */
  private burst(
    at: Vector3,
    dir: Vector3,
    spread: number,
    count: number,
    [minSpeed, maxSpeed]: [number, number],
    color: Color4,
    lifetime: number,
  ) {
    const ps = new ParticleSystem("burst", count, this.game.scene);
    ps.particleTexture = this.spark;
    ps.layerMask = Mask.World;
    ps.emitter = at.clone();
    // Directions are randomised between two vectors: a box of `spread` around `dir`.
    const d = dir.normalizeToNew();
    const jitter = new Vector3(spread, spread, spread);
    ps.createPointEmitter(d.subtract(jitter), d.add(jitter));
    ps.minEmitPower = minSpeed;
    ps.maxEmitPower = maxSpeed;
    ps.minLifeTime = lifetime * 0.6;
    ps.maxLifeTime = lifetime;
    ps.minSize = 0.08;
    ps.maxSize = 0.18;
    ps.color1 = color;
    ps.color2 = color;
    ps.colorDead = new Color4(color.r, color.g, color.b, 0);
    ps.gravity = new Vector3(0, -6, 0);
    ps.blendMode = ParticleSystem.BLENDMODE_ADD;
    ps.manualEmitCount = count;
    ps.targetStopDuration = lifetime;
    ps.disposeOnStop = true;
    ps.start();
  }
}

/** A soft round dot, drawn once on a canvas (Babylon particles need a texture). */
function sparkTexture(scene: Scene): Texture {
  const size = 64;
  const tex = new DynamicTexture("spark", size, scene, false);
  const ctx = tex.getContext();
  const g = ctx.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  g.addColorStop(0, "rgba(255,255,255,1)");
  g.addColorStop(0.4, "rgba(255,255,255,0.6)");
  g.addColorStop(1, "rgba(255,255,255,0)");
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, size, size);
  tex.hasAlpha = true;
  tex.update();
  return tex;
}
