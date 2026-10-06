#include "arena.h"

#include <godot_cpp/classes/box_shape3d.hpp>
#include <godot_cpp/classes/collision_shape3d.hpp>
#include <godot_cpp/classes/navigation_server3d.hpp>
#include <godot_cpp/classes/scene_tree.hpp>
#include <godot_cpp/classes/static_body3d.hpp>
#include <godot_cpp/core/class_db.hpp>

#include <algorithm>
#include <cmath>
#include <utility>

using namespace godot;

namespace sp {

static std::vector<CoverBlock> g_cover;

const std::vector<CoverBlock> &cover_blocks() { return g_cover; }

bool is_clear(Vector2 p, float radius) {
	const bool inside = std::max(std::abs(p.x), std::abs(p.y)) < ARENA_HALF - 0.5f - radius;
	return inside && std::all_of(g_cover.begin(), g_cover.end(), [&](const CoverBlock &b) {
		return std::max(std::abs(p.x - b.center.x), std::abs(p.y - b.center.y)) - b.half > radius;
	});
}

bool los_blocked(Vector2 a, Vector2 b) {
	const Vector2 d = b - a;
	return std::any_of(g_cover.begin(), g_cover.end(), [&](const CoverBlock &c) {
		const Vector2 lo = c.center - Vector2(c.half, c.half);
		const Vector2 hi = c.center + Vector2(c.half, c.half);
		float t0 = 0.0f, t1 = 1.0f;
		for (int i = 0; i < 2; i++) {
			if (std::abs(d[i]) < 1e-6f) {
				if (a[i] < lo[i] || a[i] > hi[i]) {
					return false;
				}
			} else {
				float ta = (lo[i] - a[i]) / d[i];
				float tb = (hi[i] - a[i]) / d[i];
				if (ta > tb) {
					std::swap(ta, tb);
				}
				t0 = std::max(t0, ta);
				t1 = std::min(t1, tb);
				if (t0 > t1) {
					return false;
				}
			}
		}
		return true;
	});
}

std::optional<Cover> find_cover(Vector2 from, Vector2 threat) {
	std::optional<Cover> best;
	for (const CoverBlock &c : g_cover) {
		const Vector2 away = (c.center - threat).normalized();
		const Vector2 spot = c.center + away * (c.half + 0.9f);
		if (!is_clear(spot, 0.5f) || !los_blocked(threat, spot) || spot.distance_to(threat) < 5.0f) {
			continue;
		}
		// Step sideways out of cover to see the threat again.
		const Vector2 side = Vector2(-away.y, away.x) * (c.half + 1.0f);
		Vector2 peek = spot;
		for (Vector2 p : { spot + side, spot - side }) {
			if (is_clear(p, 0.5f) && !los_blocked(threat, p)) {
				peek = p;
				break;
			}
		}
		if (!best || from.distance_to(spot) < from.distance_to(best->spot)) {
			best = Cover{ spot, peek };
		}
	}
	return best;
}

void Arena::_ready() {
	// Collect cover from the scene: every StaticBody3D in the "cover" group with a box shape.
	g_cover.clear();
	TypedArray<Node> nodes = get_tree()->get_nodes_in_group("cover");
	for (int i = 0; i < nodes.size(); i++) {
		auto *body = Object::cast_to<StaticBody3D>(nodes[i]);
		if (!body) {
			continue;
		}
		for (int c = 0; c < body->get_child_count(); c++) {
			auto *shape = Object::cast_to<CollisionShape3D>(body->get_child(c));
			Ref<BoxShape3D> box = shape ? Ref<BoxShape3D>(shape->get_shape()) : Ref<BoxShape3D>();
			if (box.is_valid()) {
				g_cover.push_back({ xz(body->get_global_position()), box->get_size().x * 0.5f });
			}
		}
	}
	// The arena is static: bake once, synchronously, from the colliders under this node.
	bake_navigation_mesh(false);
}

PackedVector3Array Arena::path(Vector3 from, Vector3 to) const {
	return NavigationServer3D::get_singleton()->map_get_path(get_navigation_map(), from, to, true);
}

Array Arena::find_cover_from(Vector2 from, Vector2 threat) const {
	Array out;
	if (auto c = find_cover(from, threat)) {
		out.push_back(c->spot);
		out.push_back(c->peek);
	}
	return out;
}

void Arena::_bind_methods() {
	ClassDB::bind_method(D_METHOD("path", "from", "to"), &Arena::path);
	ClassDB::bind_method(D_METHOD("is_clear", "p", "radius"), &Arena::is_clear_at);
	ClassDB::bind_method(D_METHOD("los_blocked", "a", "b"), &Arena::los_blocked_between);
	ClassDB::bind_method(D_METHOD("find_cover", "from", "threat"), &Arena::find_cover_from);
}

} // namespace sp
