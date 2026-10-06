#include "shooter.h"

#include "arena.h"

#include <godot_cpp/classes/input.hpp>
#include <godot_cpp/core/class_db.hpp>

using namespace godot;

namespace sp {

static constexpr float RADIUS = 0.35f;
static constexpr float MOVE_SPEED = 5.0f;
static constexpr float TURN_SPEED = 2.4f;
static constexpr float STEP_INTERVAL = 0.42f;
static constexpr float GRAVITY = 9.81f;

/** Tank controls, relative to the shooter's own facing. Actions are defined in project.godot. */
void Shooter::_physics_process(double delta) {
	const float dt = float(delta);
	Input *input = Input::get_singleton();
	rotate_y(input->get_axis("turn_right", "turn_left") * TURN_SPEED * dt);

	const float throttle = input->get_axis("back", "forward");
	const Vector3 planar = forward() * throttle * MOVE_SPEED;
	speed = planar.length();
	Vector3 v = get_velocity();
	v = Vector3(planar.x, is_on_floor() ? 0.0f : v.y - GRAVITY * dt, planar.z);
	set_velocity(v);
	move_and_slide();

	if (throttle != 0.0f) {
		step_timer -= dt;
		if (step_timer <= 0.0f) {
			step_timer = STEP_INTERVAL;
			emit_signal("footstep", get_global_position());
		}
	} else {
		step_timer = 0.0f;
	}
}

void Shooter::place(Vector3 pos, float yaw) {
	set_global_position(pos);
	set_global_rotation(Vector3(0, yaw, 0));
	set_velocity(Vector3());
}

Vector3 random_start(RandomNumberGenerator &rng) {
	const float r = ARENA_HALF - 2.0f;
	for (;;) {
		const Vector2 p(rng.randf_range(-r, r), rng.randf_range(-r, r));
		if (p.length() >= MIN_START_DISTANCE && is_clear(p, RADIUS + 0.3f)) {
			return Vector3(p.x, p.y, rng.randf_range(0.0f, float(Math::TAU)));
		}
	}
}

Vector3 Shooter::sample_start(const Ref<RandomNumberGenerator> &rng) {
	return random_start(**rng);
}

void Shooter::_bind_methods() {
	ClassDB::bind_static_method("Shooter", D_METHOD("sample_start", "rng"), &Shooter::sample_start);
	ClassDB::bind_method(D_METHOD("get_hp"), &Shooter::get_hp);
	ClassDB::bind_method(D_METHOD("set_hp", "hp"), &Shooter::set_hp);
	ADD_PROPERTY(PropertyInfo(Variant::FLOAT, "hp"), "set_hp", "get_hp");
	ClassDB::bind_method(D_METHOD("place", "pos", "yaw"), &Shooter::place);
	ADD_SIGNAL(MethodInfo("footstep", PropertyInfo(Variant::VECTOR3, "at")));
}

} // namespace sp
