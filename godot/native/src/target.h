// The target: root of target.tscn (CharacterBody3D → Head → Camera3D, his eyes). Perceives
// the shooter and is driven by a BehaviorTree.CPP tree defined in XML (target.cpp).
//
// Tree actions only set intent (look goal, move-to, activity); `perceive` → tick tree →
// `gaze` → `walk` run each physics frame. Preemption comes from ReactiveFallback: it
// re-checks higher-priority conditions every tick and halts a running lower branch.
#pragma once

#include "arena.h"

#include <godot_cpp/classes/camera3d.hpp>
#include <godot_cpp/classes/character_body3d.hpp>
#include <godot_cpp/classes/random_number_generator.hpp>

#include <memory>
#include <optional>

namespace BT {
class Tree;
}

namespace sp {

class Shooter;
class Arena;

constexpr int TARGET_MAX_HP = 3;
constexpr float VIEW_RANGE = 40.0f;
/** ~34°, a bit narrower than the camera so "seen" means clearly on screen. */
constexpr float VIEW_HALF_ANGLE = 0.6f;

enum class Activity { None, Scanning, Wandering, Investigating, TakingCover, Engaging };

/** How sure he is someone's out there. At 1.0 he engages, and keeps engaging until it drains to 0. */
struct Suspicion {
	float level = 0.0f;
	bool sees_shooter = false;
	bool engaged = false;
	/** Where he last saw or heard the shooter. */
	std::optional<godot::Vector3> last_known;

	void bump(float amount) {
		level = std::min(level + amount, 1.0f);
		if (level >= 1.0f) {
			engaged = true;
		}
	}
};

struct MoveTo {
	godot::Vector3 dest;
	float speed;
	/** Move without turning to face the path (side-stepping while watching a threat). */
	bool strafe;
};

class Target : public godot::CharacterBody3D {
	GDCLASS(Target, godot::CharacterBody3D)

public:
	int hp = TARGET_MAX_HP;
	Suspicion suspicion;
	Activity activity = Activity::None;
	/** Something got his attention: look toward `at` until the time runs out. */
	struct Alert {
		godot::Vector3 at;
		float remaining;
	};
	std::optional<Alert> alert;
	/** Whether he may walk around. Owned by the game. */
	bool mobile = false;

	Target();
	~Target() override;

	void _ready() override;
	void build_view_cone();
	void _physics_process(double delta) override;

	void raise_alert(godot::Vector3 at, float secs) { alert = Alert{ at, secs }; }
	void set_move_to(std::optional<MoveTo> m);
	godot::Camera3D *eyes() const { return camera; }
	godot::Vector3 eye_forward() const { return -camera->get_global_basis().get_column(2); }

	// Script-facing (tests, HUD).
	int get_hp() const { return hp; }
	void set_hp(int v) { hp = v; }
	float get_suspicion() const { return suspicion.level; }
	bool get_sees_shooter() const { return suspicion.sees_shooter; }
	bool get_engaged() const { return suspicion.engaged; }
	bool has_alert() const { return alert.has_value(); }
	godot::String get_activity() const;
	void engage(godot::Vector3 last_known);
	void place(godot::Vector3 pos, float yaw);

protected:
	static void _bind_methods();

private:
	friend struct TargetTree;

	godot::Node3D *head = nullptr;
	godot::Camera3D *camera = nullptr;
	Shooter *shooter = nullptr;
	Arena *arena = nullptr;
	std::unique_ptr<BT::Tree> tree;
	godot::Ref<godot::RandomNumberGenerator> rng;

	float yaw = 0.0f;
	float pitch = 0.0f;
	float elapsed = 0.0f;
	float dt = 0.0f;
	godot::Vector3 look_point;
	float turn_speed = 1.0f;
	std::optional<MoveTo> move_to;
	godot::PackedVector3Array route;
	Cover cover_plan{};
	struct Fighting {
		bool peeking;
		float timer;
		int hp_at_start;
	} fighting{};
	struct ScanPlan {
		float yaw;
		float dwell;
	} scan_plan{};

	void perceive(float dt);
	void gaze(float dt);
	void walk(float dt);
	godot::Vector3 forward_flat() const;
};

/** Shortest signed angle from `a` to `b`. */
float angle_diff(float a, float b);

} // namespace sp
