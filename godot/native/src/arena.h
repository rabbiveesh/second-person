// The arena scene (arena.tscn) plus the pure geometry helpers the AI uses.
//
// Unlike the Bevy/Babylon versions, cover isn't a constant table in code: it's whatever
// StaticBody3D nodes in arena.tscn are in the "cover" group. Move a crate in the editor and
// the AI's cover-finding, the navmesh and the radar map all follow.
#pragma once

#include <godot_cpp/classes/navigation_region3d.hpp>
#include <godot_cpp/variant/vector2.hpp>
#include <godot_cpp/variant/vector3.hpp>

#include <optional>
#include <vector>

namespace sp {

using godot::Vector2;
using godot::Vector3;

constexpr float ARENA_HALF = 30.0f;

/** A cover block, top-down: centre and half extent (all cover is square and taller than eyes). */
struct CoverBlock {
	Vector2 center;
	float half;
};

/** A place to hide from a threat, plus a spot to peek out from. */
struct Cover {
	Vector2 spot;
	Vector2 peek;
};

/** Cover in the loaded arena (filled by Arena::_ready). */
const std::vector<CoverBlock> &cover_blocks();
/** Is a circle of `radius` at `p` (XZ) inside the walls and clear of all cover? */
bool is_clear(Vector2 p, float radius);
/** Does any cover block sit between `a` and `b` (top-down)? */
bool los_blocked(Vector2 a, Vector2 b);
/** Nearest spot (to `from`) that's hidden from `threat` behind some block. */
std::optional<Cover> find_cover(Vector2 from, Vector2 threat);

inline Vector2 xz(Vector3 v) { return Vector2(v.x, v.z); }
inline Vector3 on_ground(Vector2 p, float y) { return Vector3(p.x, y, p.y); }

/** Root of arena.tscn: a navigation region whose children are the level's static geometry. */
class Arena : public godot::NavigationRegion3D {
	GDCLASS(Arena, godot::NavigationRegion3D)

protected:
	static void _bind_methods();

public:
	void _ready() override;

	/** Waypoints from `from` to `to` on the navmesh (empty until the map has synced). */
	godot::PackedVector3Array path(Vector3 from, Vector3 to) const;

	// Script-facing wrappers (tests).
	bool is_clear_at(Vector2 p, float radius) const { return is_clear(p, radius); }
	bool los_blocked_between(Vector2 a, Vector2 b) const { return los_blocked(a, b); }
	godot::Array find_cover_from(Vector2 from, Vector2 threat) const;
};

} // namespace sp
