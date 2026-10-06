import type { PhysicsEngineV2, Scene } from "@babylonjs/core";

// Two separate "layer" systems, as in Bevy (RenderLayers vs avian CollisionLayers).

/**
 * Render layer masks (`mesh.layerMask` & `camera.layerMask`). Babylon's default mesh mask is
 * 0x0FFFFFFF, i.e. visible to *every* camera, so anything that must stay off the radar has to
 * opt in to `World` explicitly. Bevy defaults the other way (layer 0 only).
 */
export const Mask = {
  World: 0x1,
  Radar: 0x2,
} as const;

/** Havok collision filter bits (`shape.filterMembershipMask` / `filterCollideMask`). */
export const Filter = {
  World: 1 << 0,
  Shooter: 1 << 1,
  Target: 1 << 2,
} as const;

/** The scene's Havok (physics v2) engine; `getPhysicsEngine()` is typed for v1 and v2 both. */
export function physicsOf(scene: Scene): PhysicsEngineV2 {
  return scene.getPhysicsEngine() as PhysicsEngineV2;
}
