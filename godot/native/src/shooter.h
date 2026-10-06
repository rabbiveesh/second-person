// The shooter: the body you control, seen (mostly) through the target's eyes. Root of
// shooter.tscn (a CharacterBody3D with its capsule, visor, gun and radar blips as children).
#pragma once

#include <godot_cpp/classes/character_body3d.hpp>
#include <godot_cpp/classes/random_number_generator.hpp>

namespace sp {

constexpr float SHOOTER_MAX_HP = 100.0f;
/** Minimum start distance from the target (who starts at the origin). */
constexpr float MIN_START_DISTANCE = 12.0f;

class Shooter : public godot::CharacterBody3D {
	GDCLASS(Shooter, godot::CharacterBody3D)

	float hp = SHOOTER_MAX_HP;
	float step_timer = 0.0f;
	/** Planar speed this step (the target notices movement more). */
	float speed = 0.0f;

protected:
	static void _bind_methods();

public:
	void _physics_process(double delta) override;

	float get_hp() const { return hp; }
	void set_hp(float v) { hp = v; }
	float get_speed() const { return speed; }
	godot::Vector3 forward() const { return -get_global_basis().get_column(2); }
	/** Teleport and face `yaw` (tests, spawning). */
	void place(godot::Vector3 pos, float yaw);
	/** Script-facing random_start (tests). */
	static godot::Vector3 sample_start(const godot::Ref<godot::RandomNumberGenerator> &rng);
};

/** A random start: clear of cover, away from the target, facing anywhere. Returns (x, z, yaw). */
godot::Vector3 random_start(godot::RandomNumberGenerator &rng);

} // namespace sp
