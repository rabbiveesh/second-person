// Static level geometry: ground, perimeter walls and cover, plus the pure geometry helpers
// the AI uses (no scene needed, so they're unit-testable).

import {
  Color3,
  Mesh,
  MeshBuilder,
  PhysicsAggregate,
  PhysicsShapeType,
  Scene,
  StandardMaterial,
  Vector2,
  Vector3,
} from "@babylonjs/core";
import { Filter, Mask } from "./layers";

export const ARENA_HALF = 30;
const WALL_H = 3;

/** Hand-placed cover so rounds are reproducible: crates are [x, z, half-height]. */
const CRATES: [number, number, number][] = [
  [-6, -8, 1.0],
  [7, -10, 1.2],
  [12, 4, 1.0],
  [-14, 6, 1.5],
  [-3, 14, 1.0],
  [18, -18, 1.3],
  [-20, -16, 1.0],
  [4, 20, 1.4],
];
const CRATE_HALF = 1.0;
const PILLARS: [number, number][] = [
  [-10, -2],
  [10, -2],
  [0, -18],
  [-18, 20],
  [22, 12],
];
const PILLAR_HALF = 0.6;

/** A block in the arena: centre, half extents. Shared by the 3D scene, navmesh and radar map. */
export interface Block {
  kind: "wall" | "crate" | "pillar";
  center: Vector3;
  half: Vector3;
}

export function blocks(): Block[] {
  const h = WALL_H / 2;
  const walls: Block[] = [
    [new Vector3(0, h, -ARENA_HALF), new Vector3(ARENA_HALF, h, 0.5)],
    [new Vector3(0, h, ARENA_HALF), new Vector3(ARENA_HALF, h, 0.5)],
    [new Vector3(-ARENA_HALF, h, 0), new Vector3(0.5, h, ARENA_HALF)],
    [new Vector3(ARENA_HALF, h, 0), new Vector3(0.5, h, ARENA_HALF)],
  ].map(([center, half]) => ({ kind: "wall", center, half }));
  return [
    ...walls,
    ...CRATES.map(([x, z, h]): Block => ({
      kind: "crate",
      center: new Vector3(x, h, z),
      half: new Vector3(CRATE_HALF, h, CRATE_HALF),
    })),
    ...PILLARS.map(([x, z]): Block => ({
      kind: "pillar",
      center: new Vector3(x, 2.5, z),
      half: new Vector3(PILLAR_HALF, 2.5, PILLAR_HALF),
    })),
  ];
}

export interface ArenaMeshes {
  ground: Mesh;
  /** Everything the navmesh should route around. */
  obstacles: Mesh[];
}

/** Builds the arena into the scene: meshes for the main view plus static Havok bodies. */
export function buildArena(scene: Scene): ArenaMeshes {
  const size = ARENA_HALF * 2;
  const ground = MeshBuilder.CreateBox("ground", { width: size, height: 0.2, depth: size }, scene);
  ground.position.y = -0.1;
  ground.material = material(scene, "ground", new Color3(0.36, 0.5, 0.3));
  ground.receiveShadows = true;
  staticBody(ground);

  const mats = {
    wall: material(scene, "wall", new Color3(0.5, 0.5, 0.55)),
    crate: material(scene, "crate", new Color3(0.6, 0.42, 0.25)),
    pillar: material(scene, "pillar", new Color3(0.7, 0.7, 0.72)),
  };
  const obstacles = blocks().map((b) => {
    const m = MeshBuilder.CreateBox(
      b.kind,
      { width: b.half.x * 2, height: b.half.y * 2, depth: b.half.z * 2 },
      scene,
    );
    m.position.copyFrom(b.center);
    m.material = mats[b.kind];
    m.receiveShadows = true;
    staticBody(m);
    return m;
  });
  for (const m of [ground, ...obstacles]) {
    m.layerMask = Mask.World;
    m.freezeWorldMatrix();
  }
  return { ground, obstacles };
}

function material(scene: Scene, name: string, color: Color3): StandardMaterial {
  const m = new StandardMaterial(name, scene);
  m.diffuseColor = color;
  m.specularColor = new Color3(0.05, 0.05, 0.05);
  return m;
}

function staticBody(mesh: Mesh) {
  const agg = new PhysicsAggregate(mesh, PhysicsShapeType.BOX, { mass: 0 }, mesh.getScene());
  agg.shape.filterMembershipMask = Filter.World;
}

/** Cover blocks as [centre XZ, half extent XZ]. All of them are taller than eye height. */
export function coverBlocks(): [Vector2, number][] {
  return [
    ...CRATES.map(([x, z]): [Vector2, number] => [new Vector2(x, z), CRATE_HALF]),
    ...PILLARS.map(([x, z]): [Vector2, number] => [new Vector2(x, z), PILLAR_HALF]),
  ];
}

/** Is a circle of `radius` at `p` (XZ) inside the walls and clear of all cover? */
export function isClear(p: Vector2, radius: number): boolean {
  const inside = Math.max(Math.abs(p.x), Math.abs(p.y)) < ARENA_HALF - 0.5 - radius;
  return (
    inside &&
    coverBlocks().every(
      ([c, half]) => Math.max(Math.abs(p.x - c.x) - half, Math.abs(p.y - c.y) - half) > radius,
    )
  );
}

/** Does any cover block sit between `a` and `b` (top-down)? Slab test against each block's square. */
export function losBlocked(a: Vector2, b: Vector2): boolean {
  const d = b.subtract(a);
  return coverBlocks().some(([c, half]) => {
    const lo = [c.x - half, c.y - half];
    const hi = [c.x + half, c.y + half];
    const av = [a.x, a.y];
    const dv = [d.x, d.y];
    let t0 = 0;
    let t1 = 1;
    for (let i = 0; i < 2; i++) {
      if (Math.abs(dv[i]) < 1e-6) {
        if (av[i] < lo[i] || av[i] > hi[i]) return false;
      } else {
        let ta = (lo[i] - av[i]) / dv[i];
        let tb = (hi[i] - av[i]) / dv[i];
        if (ta > tb) [ta, tb] = [tb, ta];
        t0 = Math.max(t0, ta);
        t1 = Math.min(t1, tb);
        if (t0 > t1) return false;
      }
    }
    return true;
  });
}

/** A place to hide from a threat, plus a spot to peek out from. */
export interface Cover {
  spot: Vector2;
  peek: Vector2;
}

/** Nearest spot (to `from`) that's hidden from `threat` behind some block. */
export function findCover(from: Vector2, threat: Vector2): Cover | undefined {
  let best: Cover | undefined;
  for (const [c, half] of coverBlocks()) {
    const away = c.subtract(threat).normalize();
    const spot = c.add(away.scale(half + 0.9));
    if (!isClear(spot, 0.5) || !losBlocked(threat, spot) || Vector2.Distance(spot, threat) < 5) {
      continue;
    }
    // Step sideways out of cover to see the threat again.
    const side = new Vector2(-away.y, away.x).scale(half + 1.0);
    const peek =
      [spot.add(side), spot.subtract(side)].find((p) => isClear(p, 0.5) && !losBlocked(threat, p)) ??
      spot;
    if (!best || Vector2.Distance(from, spot) < Vector2.Distance(from, best.spot)) {
      best = { spot, peek };
    }
  }
  return best;
}

/** XZ of a 3D point. */
export const xz = (v: Vector3) => new Vector2(v.x, v.z);
