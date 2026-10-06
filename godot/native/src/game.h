// Root of main.tscn: round lifecycle, global input, combat, and the game's signals. Fx, audio,
// radar and HUD connect to those signals (Godot's built-in events) rather than being called.
#pragma once

#include <godot_cpp/classes/node3d.hpp>
#include <godot_cpp/classes/packed_scene.hpp>
#include <godot_cpp/classes/random_number_generator.hpp>
#include <godot_cpp/classes/rigid_body3d.hpp>

namespace sp {

class Shooter;
class Target;

enum class GameState { Playing, Won, Lost };
/** The difficulty knob (Tab cycles). */
enum class RadarMode { Full, Sonar, Off };

class Game : public godot::Node3D {
	GDCLASS(Game, godot::Node3D)

public:
	GameState state = GameState::Playing;
	RadarMode radar_mode = RadarMode::Sonar;
	bool target_mobile = false;

	Game();
	void _ready() override;
	void _physics_process(double delta) override;

	Shooter *get_shooter() const { return shooter; }
	Target *get_target() const { return target; }
	void start_round();

	// Script-facing (tests, HUD).
	godot::String get_state() const;
	godot::String get_radar_mode() const;
	bool get_target_mobile() const { return target_mobile; }
	int bullet_count() const;
	void set_seed(int64_t seed) { rng->set_seed(seed); }

protected:
	static void _bind_methods();

private:
	godot::Node3D *round = nullptr;
	Shooter *shooter = nullptr;
	Target *target = nullptr;
	godot::Ref<godot::PackedScene> shooter_scene, target_scene, bullet_scene;
	godot::Ref<godot::RandomNumberGenerator> rng;
	float cooldown = 0.0f;
	float return_fire_timer = 0.0f;

	void fire(float dt);
	void bullet_hit(godot::Node *body, godot::RigidBody3D *bullet);
	void return_fire(float dt);
	void end(GameState s);
	void relay_footstep(godot::Vector3 at);
};

} // namespace sp
