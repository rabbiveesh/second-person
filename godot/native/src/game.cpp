#include "game.h"

#include "shooter.h"
#include "target.h"

#include <godot_cpp/classes/input.hpp>
#include <godot_cpp/classes/resource_loader.hpp>
#include <godot_cpp/core/class_db.hpp>

using namespace godot;

namespace sp {

static constexpr float BULLET_SPEED = 45.0f;
static constexpr float FIRE_COOLDOWN = 0.3f;
static constexpr float HEARING_RANGE = 18.0f;
static constexpr float NEAR_MISS_RANGE = 7.0f;
static constexpr float RETURN_FIRE_INTERVAL = 0.8f;
static constexpr float RETURN_FIRE_DAMAGE = 8.0f;
static const Vector3 MUZZLE(0.3f, 0.15f, -0.9f);

Game::Game() {
	rng.instantiate();
}

void Game::_ready() {
	ResourceLoader *loader = ResourceLoader::get_singleton();
	shooter_scene = loader->load("res://scenes/shooter.tscn");
	target_scene = loader->load("res://scenes/target.tscn");
	bullet_scene = loader->load("res://scenes/bullet.tscn");
	round = get_node<Node3D>("Round");
	start_round();
}

void Game::start_round() {
	// Everything per-round lives under Round; freeing its children is the whole cleanup.
	for (int i = round->get_child_count() - 1; i >= 0; i--) {
		Node *child = round->get_child(i);
		round->remove_child(child);
		child->queue_free();
	}
	round->set_process_mode(PROCESS_MODE_INHERIT);
	cooldown = return_fire_timer = 0.0f;

	target = Object::cast_to<Target>(target_scene->instantiate());
	round->add_child(target);
	shooter = Object::cast_to<Shooter>(shooter_scene->instantiate());
	round->add_child(shooter);
	const Vector3 start = random_start(**rng);
	shooter->place(Vector3(start.x, 0.9f, start.y), start.z);
	shooter->connect("footstep", callable_mp(this, &Game::relay_footstep));
	target->mobile = target_mobile;
	state = GameState::Playing;
	emit_signal("round_started");
}

void Game::relay_footstep(Vector3 at) {
	emit_signal("footstep", at);
}

void Game::_physics_process(double delta) {
	Input *input = Input::get_singleton();
	if (input->is_action_just_pressed("cycle_radar")) {
		radar_mode = RadarMode((int(radar_mode) + 1) % 3);
	}
	if (input->is_action_just_pressed("restart") && state != GameState::Playing) {
		start_round();
	}
	if (input->is_action_just_pressed("toggle_mobility")) {
		target_mobile = !target_mobile;
	}
	if (state != GameState::Playing) {
		return;
	}
	target->mobile = target_mobile;
	fire(float(delta));
	return_fire(float(delta));
	if (target->hp == 0) {
		end(GameState::Won);
	} else if (shooter->get_hp() <= 0.0f) {
		end(GameState::Lost);
	}
}

void Game::fire(float dt) {
	cooldown = std::max(cooldown - dt, 0.0f);
	if (!Input::get_singleton()->is_action_just_pressed("fire") || cooldown > 0.0f) {
		return;
	}
	cooldown = FIRE_COOLDOWN;

	const Vector3 forward = shooter->forward();
	const Vector3 muzzle = shooter->get_global_transform().xform(MUZZLE);
	auto *bullet = Object::cast_to<RigidBody3D>(bullet_scene->instantiate());
	bullet->set_meta("origin", shooter->get_global_position());
	round->add_child(bullet);
	bullet->set_global_position(muzzle);
	bullet->set_linear_velocity(forward * BULLET_SPEED);
	bullet->connect("body_entered", callable_mp(this, &Game::bullet_hit).bind(bullet));
	emit_signal("gunshot", muzzle, forward);

	// Gunshots are loud.
	if (target->get_global_position().distance_to(shooter->get_global_position()) < HEARING_RANGE) {
		target->suspicion.bump(0.25f);
		target->suspicion.last_known = shooter->get_global_position();
		target->raise_alert(shooter->get_global_position(), 3.0f);
	}
}

void Game::bullet_hit(Node *body, RigidBody3D *bullet) {
	if (bullet->is_queued_for_deletion()) {
		return;
	}
	const Vector3 origin = bullet->get_meta("origin");
	const Vector3 at = bullet->get_global_position();
	bullet->queue_free();
	Suspicion &s = target->suspicion;
	if (body == target) {
		target->hp = std::max(target->hp - 1, 0);
		s.bump(0.6f);
		s.last_known = origin;
		target->raise_alert(origin, 3.0f);
		emit_signal("target_hit", target->get_global_position());
		return;
	}
	emit_signal("bullet_impact", at);
	// Near miss: he hears the impact and looks toward where it came from.
	if (target->get_global_position().distance_to(at) < NEAR_MISS_RANGE) {
		s.bump(0.3f);
		s.last_known = origin;
		target->raise_alert(origin, 2.5f);
	}
}

/** While engaging and able to see the shooter, the target fires back (hitscan). */
void Game::return_fire(float dt) {
	if (target->activity != Activity::Engaging || !target->suspicion.sees_shooter) {
		return_fire_timer = 0.0f;
		return;
	}
	return_fire_timer += dt;
	if (return_fire_timer < RETURN_FIRE_INTERVAL) {
		return;
	}
	return_fire_timer -= RETURN_FIRE_INTERVAL;
	shooter->set_hp(std::max(shooter->get_hp() - RETURN_FIRE_DAMAGE, 0.0f));
	// Start the tracer just below the eyes so it's visible from his own view.
	const Transform3D eyes = target->eyes()->get_global_transform();
	const Vector3 from = eyes.origin - eyes.basis.get_column(1) * 0.3f + eyes.basis.get_column(0) * 0.2f;
	emit_signal("shooter_hit", from, shooter->get_global_position());
}

void Game::end(GameState s) {
	state = s;
	// Freeze the round (physics bodies, AI) while the banner is up.
	round->set_process_mode(PROCESS_MODE_DISABLED);
}

String Game::get_state() const {
	return state == GameState::Playing ? "Playing" : state == GameState::Won ? "Won" : "Lost";
}

String Game::get_radar_mode() const {
	return radar_mode == RadarMode::Full ? "Full" : radar_mode == RadarMode::Sonar ? "Sonar" : "Off";
}

int Game::bullet_count() const {
	int n = 0;
	for (int i = 0; i < round->get_child_count(); i++) {
		auto *b = Object::cast_to<RigidBody3D>(round->get_child(i));
		n += b && !b->is_queued_for_deletion();
	}
	return n;
}

void Game::_bind_methods() {
	ClassDB::bind_method(D_METHOD("get_shooter"), &Game::get_shooter);
	ClassDB::bind_method(D_METHOD("get_target"), &Game::get_target);
	ClassDB::bind_method(D_METHOD("get_state"), &Game::get_state);
	ClassDB::bind_method(D_METHOD("get_radar_mode"), &Game::get_radar_mode);
	ClassDB::bind_method(D_METHOD("get_target_mobile"), &Game::get_target_mobile);
	ClassDB::bind_method(D_METHOD("bullet_count"), &Game::bullet_count);
	ClassDB::bind_method(D_METHOD("set_seed", "seed"), &Game::set_seed);
	ClassDB::bind_method(D_METHOD("start_round"), &Game::start_round);

	ADD_SIGNAL(MethodInfo("round_started"));
	ADD_SIGNAL(MethodInfo("gunshot", PropertyInfo(Variant::VECTOR3, "muzzle"), PropertyInfo(Variant::VECTOR3, "dir")));
	ADD_SIGNAL(MethodInfo("bullet_impact", PropertyInfo(Variant::VECTOR3, "at")));
	ADD_SIGNAL(MethodInfo("target_hit", PropertyInfo(Variant::VECTOR3, "at")));
	ADD_SIGNAL(MethodInfo("shooter_hit", PropertyInfo(Variant::VECTOR3, "from"), PropertyInfo(Variant::VECTOR3, "to")));
	ADD_SIGNAL(MethodInfo("footstep", PropertyInfo(Variant::VECTOR3, "at")));
}

} // namespace sp
