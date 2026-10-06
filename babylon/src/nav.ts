// Navigation: a Recast/Detour navmesh (Babylon's navigation addon over recast-navigation-js),
// voxelised from the arena's own meshes. Bevy's version (vleue_navigator) triangulates
// collider outlines in 2D; Recast rasterises the 3D geometry instead.

import { CreateNavigationPluginAsync, type RecastNavigationJSPluginV2 } from "@babylonjs/addons/navigation";
import { Vector3, type Mesh } from "@babylonjs/core";
import * as RecastCore from "@recast-navigation/core";
import * as RecastGenerators from "@recast-navigation/generators";

const CELL = 0.2;
/** Clearance kept from walls and cover (matches the Bevy agent radius). */
const AGENT_RADIUS = 0.6;

export class Nav {
  private constructor(private plugin: RecastNavigationJSPluginV2) {}

  /**
   * Builds the navmesh synchronously from `meshes`. The addon would otherwise fetch Recast
   * from unpkg at runtime; we inject the npm copy so it's bundled and works offline/in tests.
   */
  static async build(meshes: Mesh[]): Promise<Nav> {
    await RecastCore.init();
    const plugin = await CreateNavigationPluginAsync({
      instance: { ...RecastCore, ...RecastGenerators } as never,
    });
    plugin.createNavMesh(meshes, {
      cs: CELL,
      ch: CELL,
      walkableSlopeAngle: 35,
      // Recast wants these in voxels, not metres.
      walkableHeight: Math.ceil(1.8 / CELL),
      walkableClimb: Math.floor(0.4 / CELL),
      walkableRadius: Math.ceil(AGENT_RADIUS / CELL),
      maxEdgeLen: 12 / CELL,
      maxSimplificationError: 1.3,
      minRegionArea: 8,
      mergeRegionArea: 20,
      maxVertsPerPoly: 6,
      detailSampleDist: 6,
      detailSampleMaxError: 1,
      // The arena never changes: a plain solo navmesh, no tile cache for dynamic obstacles.
      maxObstacles: 0,
    });
    return new Nav(plugin);
  }

  /** Straight-line waypoints from `from` to `to` (y on the ground), or empty if there's no path. */
  path(from: Vector3, to: Vector3): Vector3[] {
    return this.plugin.computePath(
      new Vector3(from.x, 0, from.z),
      new Vector3(to.x, 0, to.z),
      { halfExtents: { x: 2, y: 4, z: 2 } },
    );
  }

  dispose() {
    this.plugin.dispose();
  }
}
