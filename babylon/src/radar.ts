// Radar: a top-down orthographic camera in a corner viewport. It only sees the radar layer:
// a flat map of the arena plus contacts. Actors live on the world layer, so the radar can
// never see them directly, only via blips and sonar contacts.
//
// Unlike Bevy (where the radar camera re-rendered the real arena under its own ambient light),
// the radar draws a separate flat map. Babylon has no per-camera light override, and sharing
// the lit ground would leak the actors' shadows onto the radar.
//
// Modes (`game.radarMode`, Tab cycles):
// - Full: live blips, shooter heading, target view cone.
// - Sonar (default): a sweep every 2s spawns fading contacts; gunshots ping too.
// - Off: no radar.

import {
  Color3,
  FreeCamera,
  type Mesh,
  MeshBuilder,
  type Scene,
  type StandardMaterial,
  Vector3,
  Viewport,
} from "@babylonjs/core";
import { ARENA_HALF, blocks } from "./arena";
import type { Game } from "./game";
import { Mask } from "./layers";
import { unlit } from "./shooter";

/** Fraction of the window's shorter side that the radar occupies. */
const RADAR_FRACTION = 0.34;
const RADAR_MARGIN = 16;
const SONAR_PERIOD = 2;
const CONTACT_FADE = 1.8;
const SWEEP_TIME = 0.9;
const BLIP_HEIGHT = 4;

interface Fading {
  mesh: Mesh;
  material: StandardMaterial;
  age: number;
  life: number;
  baseAlpha: number;
  growTo?: number;
}

export class Radar {
  readonly camera: FreeCamera;
  /** Where the radar is on screen, in render pixels from the top-left, so the HUD can frame it. */
  rect = { left: 0, top: 0, size: 0 };
  private applied: string | undefined;
  private fading: Fading[] = [];
  private sonarTimer = 0;

  constructor(
    private game: Game,
    private hudCamera: FreeCamera,
  ) {
    const scene = game.scene;
    const cam = new FreeCamera("radar", new Vector3(0, 60, 0), scene);
    cam.mode = FreeCamera.ORTHOGRAPHIC_CAMERA;
    const r = ARENA_HALF + 1;
    cam.orthoLeft = -r;
    cam.orthoRight = r;
    cam.orthoTop = r;
    cam.orthoBottom = -r;
    // North (-Z) is up on the radar.
    cam.upVector = new Vector3(0, 0, -1);
    cam.setTarget(Vector3.Zero());
    cam.layerMask = Mask.Radar;
    cam.inputs.clear();
    this.camera = cam;
    this.buildMap(scene);

    game.events.gunshot.add(({ muzzle }) => {
      if (game.radarMode === "Sonar") this.contact(muzzle, new Color3(1, 0.8, 0.3));
    });
    game.events.roundStarted.add(() => this.reset());
    scene.onBeforeRenderObservable.add(() => this.update(scene.getEngine().getDeltaTime() / 1000));
    this.reset();
  }

  private buildMap(scene: Scene) {
    const bg = MeshBuilder.CreateGround("radar bg", { width: 200, height: 200 }, scene);
    bg.material = unlit(scene, "radar bg", new Color3(0.02, 0.07, 0.04));
    const floor = MeshBuilder.CreateGround("radar floor", { width: ARENA_HALF * 2, height: ARENA_HALF * 2 }, scene);
    floor.position.y = 0.5;
    floor.material = unlit(scene, "radar floor", new Color3(0.09, 0.2, 0.11));
    const mats = {
      wall: unlit(scene, "radar wall", new Color3(0.35, 0.42, 0.38)),
      crate: unlit(scene, "radar crate", new Color3(0.45, 0.34, 0.2)),
      pillar: unlit(scene, "radar pillar", new Color3(0.55, 0.58, 0.56)),
    };
    const map = [bg, floor];
    for (const b of blocks()) {
      const m = MeshBuilder.CreateGround(`radar ${b.kind}`, { width: b.half.x * 2, height: b.half.z * 2 }, scene);
      m.position.set(b.center.x, 1, b.center.z);
      m.material = mats[b.kind];
      map.push(m);
    }
    for (const m of map) {
      m.layerMask = Mask.Radar;
      m.isPickable = false;
      m.freezeWorldMatrix();
    }
  }

  private reset() {
    for (const f of this.fading) f.mesh.dispose();
    this.fading = [];
    this.sonarTimer = 0;
    this.applyMode();
  }

  private applyMode() {
    const { game } = this;
    const key = `${game.radarMode}:${game.target.camera.uniqueId}`;
    if (key === this.applied) return;
    this.applied = key;
    const scene = game.scene;
    const main = game.target.camera;
    const on = game.radarMode !== "Off";
    scene.activeCameras = on ? [main, this.camera, this.hudCamera] : [main, this.hudCamera];
    for (const m of [...game.shooter.liveBlips, ...game.target.liveBlips]) {
      m.setEnabled(game.radarMode === "Full");
    }
  }

  private update(dt: number) {
    const { game } = this;
    this.applyMode();
    this.fitViewport();
    if (game.state === "Playing" && game.radarMode === "Sonar") {
      this.sonarTimer += dt;
      if (this.sonarTimer >= SONAR_PERIOD) {
        this.sonarTimer -= SONAR_PERIOD;
        for (const actor of [game.shooter, game.target]) this.contact(actor.position, actor.radarColor);
        this.sweep();
      }
    }
    for (let i = this.fading.length - 1; i >= 0; i--) {
      const f = this.fading[i];
      f.age += dt;
      const k = Math.min(f.age / f.life, 1);
      f.material.alpha = f.baseAlpha * (1 - k);
      if (f.growTo) f.mesh.scaling.setAll(f.growTo * Math.max(k, 0.01));
      if (k >= 1) {
        f.mesh.dispose(false, true);
        this.fading.splice(i, 1);
      }
    }
  }

  private contact(at: Vector3, color: Color3) {
    const scene = this.game.scene;
    const mesh = MeshBuilder.CreateSphere("sonar contact", { diameter: 1.8, segments: 8 }, scene);
    mesh.position.set(at.x, BLIP_HEIGHT, at.z);
    this.addFading(mesh, unlit(scene, "contact", color), CONTACT_FADE, 1);
  }

  private sweep() {
    const scene = this.game.scene;
    const mesh = MeshBuilder.CreateTorus("sonar sweep", { diameter: 2, thickness: 0.05, tessellation: 64 }, scene);
    mesh.position.y = BLIP_HEIGHT;
    this.addFading(mesh, unlit(scene, "sweep", new Color3(0.3, 1, 0.5)), SWEEP_TIME, 0.6, ARENA_HALF * 1.45);
  }

  private addFading(mesh: Mesh, material: StandardMaterial, life: number, baseAlpha: number, growTo?: number) {
    material.alpha = baseAlpha;
    mesh.material = material;
    mesh.layerMask = Mask.Radar;
    mesh.isPickable = false;
    this.fading.push({ mesh, material, age: 0, life, baseAlpha, growTo });
  }

  /** Keep the radar square in the bottom-right corner, whatever the window size. */
  private fitViewport() {
    const engine = this.game.scene.getEngine();
    const w = engine.getRenderWidth();
    const h = engine.getRenderHeight();
    const side = Math.min(w, h) * RADAR_FRACTION;
    const dpr = w / (engine.getRenderingCanvas()?.clientWidth || w);
    const margin = RADAR_MARGIN * dpr;
    // Viewport is normalised, origin bottom-left.
    const vp = new Viewport((w - side - margin) / w, margin / h, side / w, side / h);
    const cur = this.camera.viewport;
    if (cur.x !== vp.x || cur.y !== vp.y || cur.width !== vp.width || cur.height !== vp.height) {
      this.camera.viewport = vp;
    }
    this.rect = { left: w - side - margin, top: h - side - margin, size: side };
  }
}
