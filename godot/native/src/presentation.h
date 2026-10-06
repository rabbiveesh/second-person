// Everything that only matters with a screen and speakers. Each is a node placed in
// main.tscn that connects to the Game's signals in _ready.
#pragma once

#include <godot_cpp/classes/audio_stream.hpp>
#include <godot_cpp/classes/control.hpp>
#include <godot_cpp/classes/node3d.hpp>
#include <godot_cpp/classes/panel_container.hpp>
#include <godot_cpp/classes/mesh_instance3d.hpp>
#include <godot_cpp/classes/standard_material3d.hpp>

#include <vector>

namespace sp {

class Game;

/**
 * Radar: a PanelContainer holding a SubViewport whose orthographic camera only sees render
 * layer 2. That layer holds a flat map built from the arena's colliders, actor blips (Full
 * mode) and fading sonar contacts. Actors' bodies are on layer 1, so the radar can never see
 * them directly. The map is unshaded, so the sun's shadows can't leak positions onto it.
 *
 * Modes: Full (live blips, heading, view cone), Sonar (default: a sweep every 2s spawns fading
 * contacts; gunshots ping too), Off.
 */
class Radar : public godot::PanelContainer {
	GDCLASS(Radar, godot::PanelContainer)

	struct Fading {
		godot::MeshInstance3D *mesh;
		godot::Ref<godot::StandardMaterial3D> material;
		float age, life, base_alpha, grow_to;
	};

	Game *game = nullptr;
	godot::Node3D *world = nullptr;
	std::vector<Fading> fading;
	float sonar_timer = 0.0f;

	void build_map();
	void contact(godot::Vector3 at, godot::Color color);
	void sweep();
	void add_fading(godot::MeshInstance3D *mesh, godot::Color color, float life, float base_alpha, float grow_to);
	void reset();
	void on_gunshot(godot::Vector3 muzzle, godot::Vector3 dir);

protected:
	static void _bind_methods() {}

public:
	void _ready() override;
	void _process(double delta) override;
};

/** HUD: hud.tscn's control tree, refreshed from game state every frame. */
class Hud : public godot::Control {
	GDCLASS(Hud, godot::Control)

	Game *game = nullptr;
	float flash_hurt = 0.0f, flash_hit = 0.0f;
	void on_shooter_hit(godot::Vector3 from, godot::Vector3 to);
	void on_target_hit(godot::Vector3 at);

protected:
	static void _bind_methods() {}

public:
	void _ready() override;
	void _process(double delta) override;
};

/**
 * Particle bursts (CPUParticles3D, so it runs on the Compatibility renderer too), the muzzle
 * light and return-fire tracers. The muzzle flash is a navigation aid: it lights up the
 * shooter's surroundings even when he's off-screen.
 */
class Fx : public godot::Node3D {
	GDCLASS(Fx, godot::Node3D)

	void burst(godot::Vector3 at, godot::Vector3 dir, float spread_deg, int count, float min_speed, float max_speed,
			godot::Color color, float lifetime);
	void expire(godot::Node *node, float secs);
	void on_gunshot(godot::Vector3 muzzle, godot::Vector3 dir);
	void on_impact(godot::Vector3 at);
	void on_target_hit(godot::Vector3 at);
	void on_shooter_hit(godot::Vector3 from, godot::Vector3 to);

protected:
	static void _bind_methods() {}

public:
	void _ready() override;
};

/**
 * Spatial one-shots. There's no explicit listener: Godot hears from the current camera, which
 * is the target's eyes, so you hear your own footsteps and shots from *his* position.
 */
class Audio : public godot::Node3D {
	GDCLASS(Audio, godot::Node3D)

	Game *game = nullptr;
	godot::Ref<godot::AudioStream> shot, impact, target_hit, return_fire, step;
	void play(const godot::Ref<godot::AudioStream> &stream, godot::Vector3 at, float radius);
	void on_gunshot(godot::Vector3 at, godot::Vector3 dir);
	void on_impact(godot::Vector3 at);
	void on_target_hit(godot::Vector3 at);
	void on_shooter_hit(godot::Vector3 from, godot::Vector3 to);
	void on_footstep(godot::Vector3 at);

protected:
	static void _bind_methods() {}

public:
	void _ready() override;
};

} // namespace sp
