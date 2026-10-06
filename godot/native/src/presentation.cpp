#include "presentation.h"

#include <godot_cpp/classes/camera3d.hpp>

#include "arena.h"
#include "game.h"
#include "layers.h"
#include "shooter.h"
#include "target.h"

#include <godot_cpp/classes/audio_stream_player3d.hpp>
#include <godot_cpp/classes/box_shape3d.hpp>
#include <godot_cpp/classes/collision_shape3d.hpp>
#include <godot_cpp/classes/color_rect.hpp>
#include <godot_cpp/classes/cpu_particles3d.hpp>
#include <godot_cpp/classes/gradient.hpp>
#include <godot_cpp/classes/immediate_mesh.hpp>
#include <godot_cpp/classes/label.hpp>
#include <godot_cpp/classes/mesh_instance3d.hpp>
#include <godot_cpp/classes/omni_light3d.hpp>
#include <godot_cpp/classes/plane_mesh.hpp>
#include <godot_cpp/classes/progress_bar.hpp>
#include <godot_cpp/classes/resource_loader.hpp>
#include <godot_cpp/classes/scene_tree.hpp>
#include <godot_cpp/classes/scene_tree_timer.hpp>
#include <godot_cpp/classes/sphere_mesh.hpp>
#include <godot_cpp/classes/static_body3d.hpp>
#include <godot_cpp/classes/style_box_flat.hpp>
#include <godot_cpp/classes/sub_viewport.hpp>
#include <godot_cpp/classes/torus_mesh.hpp>
#include <godot_cpp/classes/viewport.hpp>
#include <godot_cpp/classes/window.hpp>

using namespace godot;

namespace sp {

static Game *find_game(Node *from) {
	return Object::cast_to<Game>(from->get_tree()->get_first_node_in_group("game"));
}

static Ref<StandardMaterial3D> unshaded(Color c) {
	Ref<StandardMaterial3D> m;
	m.instantiate();
	m->set_shading_mode(BaseMaterial3D::SHADING_MODE_UNSHADED);
	m->set_albedo(c);
	if (c.a < 1.0f) {
		m->set_transparency(BaseMaterial3D::TRANSPARENCY_ALPHA);
	}
	m->set_cull_mode(BaseMaterial3D::CULL_DISABLED);
	return m;
}

// ---------------------------------------------------------------------------
// Radar
// ---------------------------------------------------------------------------

static constexpr float RADAR_FRACTION = 0.34f;
static constexpr float RADAR_MARGIN = 16.0f;
static constexpr float SONAR_PERIOD = 2.0f;
static constexpr float CONTACT_FADE = 1.8f;
static constexpr float BLIP_HEIGHT = 4.0f;

void Radar::_ready() {
	game = find_game(this);
	world = get_node<Node3D>("View/Viewport/World");
	// Straight down, north (-Z) up.
	world->get_node<Camera3D>("Camera")->look_at_from_position(Vector3(0, 60, 0), Vector3(), Vector3(0, 0, -1));
	build_map();
	game->connect("gunshot", callable_mp(this, &Radar::on_gunshot));
	game->connect("round_started", callable_mp(this, &Radar::reset));
}

/** A flat, unshaded copy of the arena's colliders: ground, walls and cover. */
void Radar::build_map() {
	Node *arena = get_tree()->get_first_node_in_group("arena");
	for (int i = 0; i < arena->get_child_count(); i++) {
		auto *body = Object::cast_to<StaticBody3D>(arena->get_child(i));
		auto *shape = body ? body->get_node<CollisionShape3D>("Shape") : nullptr;
		Ref<BoxShape3D> box = shape ? Ref<BoxShape3D>(shape->get_shape()) : Ref<BoxShape3D>();
		if (box.is_null()) {
			continue;
		}
		const String name = body->get_name();
		const Color color = name.begins_with("Ground") ? Color(0.09f, 0.2f, 0.11f)
				: name.begins_with("Crate")             ? Color(0.45f, 0.34f, 0.2f)
				: name.begins_with("Pillar")            ? Color(0.55f, 0.58f, 0.56f)
														: Color(0.35f, 0.42f, 0.38f);
		Ref<PlaneMesh> plane;
		plane.instantiate();
		plane->set_size(Vector2(box->get_size().x, box->get_size().z));
		auto *m = memnew(MeshInstance3D);
		m->set_mesh(plane);
		m->set_material_override(unshaded(color));
		m->set_layer_mask(Render::Radar);
		world->add_child(m);
		const Vector3 p = body->get_global_position();
		m->set_position(Vector3(p.x, name.begins_with("Ground") ? 0.5f : 1.0f, p.z));
	}
}

// Gunshots give away the shooter's position on the radar.
void Radar::on_gunshot(Vector3 muzzle, Vector3) {
	if (game->radar_mode == RadarMode::Sonar) {
		contact(muzzle, Color(1.0f, 0.8f, 0.3f));
	}
}

void Radar::reset() {
	for (auto &f : fading) {
		f.mesh->queue_free();
	}
	fading.clear();
	sonar_timer = 0.0f;
}

void Radar::_process(double delta) {
	const float dt = float(delta);
	set_visible(game->radar_mode != RadarMode::Off);

	// Keep the radar square in the bottom-right corner, whatever the window size.
	const Vector2 screen = get_viewport_rect().size;
	const float side = std::min(screen.x, screen.y) * RADAR_FRACTION;
	set_size(Vector2(side, side));
	set_position(screen - Vector2(side, side) - Vector2(RADAR_MARGIN, RADAR_MARGIN));
	get_node<Label>("Label")->set_text(String("RADAR · ") + game->get_radar_mode().to_upper());

	// Live blips (children of the actor scenes in the "live_blip" group) only in Full mode.
	TypedArray<Node> blips = get_tree()->get_nodes_in_group("live_blip");
	for (int i = 0; i < blips.size(); i++) {
		Object::cast_to<Node3D>(blips[i])->set_visible(game->radar_mode == RadarMode::Full);
	}

	if (game->state == GameState::Playing && game->radar_mode == RadarMode::Sonar) {
		sonar_timer += dt;
		if (sonar_timer >= SONAR_PERIOD) {
			sonar_timer -= SONAR_PERIOD;
			contact(game->get_shooter()->get_global_position(), Color(0.2f, 1.0f, 0.3f));
			contact(game->get_target()->get_global_position(), Color(1.0f, 0.2f, 0.2f));
			sweep();
		}
	}
	for (size_t i = fading.size(); i-- > 0;) {
		Fading &f = fading[i];
		f.age += dt;
		const float k = std::min(f.age / f.life, 1.0f);
		Color c = f.material->get_albedo();
		c.a = f.base_alpha * (1.0f - k);
		f.material->set_albedo(c);
		if (f.grow_to > 0.0f) {
			f.mesh->set_scale(Vector3(1, 1, 1) * f.grow_to * std::max(k, 0.01f));
		}
		if (k >= 1.0f) {
			f.mesh->queue_free();
			fading.erase(fading.begin() + i);
		}
	}
}

void Radar::contact(Vector3 at, Color color) {
	Ref<SphereMesh> sphere;
	sphere.instantiate();
	sphere->set_radius(0.9f);
	sphere->set_height(1.8f);
	auto *m = memnew(MeshInstance3D);
	m->set_mesh(sphere);
	m->set_position(Vector3(at.x, BLIP_HEIGHT, at.z));
	add_fading(m, color, CONTACT_FADE, 1.0f, 0.0f);
}

void Radar::sweep() {
	Ref<TorusMesh> ring;
	ring.instantiate();
	ring->set_inner_radius(0.95f);
	ring->set_outer_radius(1.0f);
	auto *m = memnew(MeshInstance3D);
	m->set_mesh(ring);
	m->set_position(Vector3(0, BLIP_HEIGHT, 0));
	add_fading(m, Color(0.3f, 1.0f, 0.5f), 0.9f, 0.6f, ARENA_HALF * 1.45f);
}

void Radar::add_fading(MeshInstance3D *m, Color color, float life, float base_alpha, float grow_to) {
	color.a = base_alpha;
	Ref<StandardMaterial3D> mat = unshaded(color);
	mat->set_transparency(BaseMaterial3D::TRANSPARENCY_ALPHA);
	m->set_material_override(mat);
	m->set_layer_mask(Render::Radar);
	world->add_child(m);
	fading.push_back({ m, mat, 0.0f, life, base_alpha, grow_to });
}

// ---------------------------------------------------------------------------
// HUD
// ---------------------------------------------------------------------------

void Hud::_ready() {
	game = find_game(this);
	game->connect("shooter_hit", callable_mp(this, &Hud::on_shooter_hit));
	game->connect("target_hit", callable_mp(this, &Hud::on_target_hit));
}

void Hud::on_shooter_hit(Vector3, Vector3) { flash_hurt = 0.6f; }
void Hud::on_target_hit(Vector3) { flash_hit = 0.7f; }

void Hud::_process(double delta) {
	const float decay = float(delta) * 2.5f;
	flash_hurt = std::max(flash_hurt - decay, 0.0f);
	flash_hit = std::max(flash_hit - decay, 0.0f);
	auto *flash = get_node<ColorRect>("%Flash");
	flash->set_visible(flash_hurt > 0.0f || flash_hit > 0.0f);
	flash->set_color(flash_hurt > 0.0f ? Color(1, 0, 0, flash_hurt * 120 / 255) : Color(1, 1, 1, flash_hit * 160 / 255));

	Shooter *shooter = game->get_shooter();
	Target *target = game->get_target();
	const Suspicion &s = target->suspicion;
	auto *hp = get_node<ProgressBar>("%Hp");
	hp->set_value(shooter->get_hp());
	get_node<Label>("%HpText")->set_text(String::num(shooter->get_hp(), 0) + " HP");

	String hearts;
	for (int i = 0; i < TARGET_MAX_HP; i++) {
		hearts += i < target->hp ? String::utf8("♥") : String::utf8("♡");
	}
	get_node<Label>("%TargetLine")->set_text("TARGET  " + hearts);
	auto *bar = get_node<ProgressBar>("%Suspicion");
	bar->set_value(s.level);
	Ref<StyleBoxFlat> fill = bar->get_theme_stylebox("fill");
	fill->set_bg_color(s.engaged ? Color::from_rgba8(220, 50, 40) : Color::from_rgba8(220, 170, 40));
	get_node<Label>("%SuspicionText")->set_text(s.engaged ? "ENGAGING" : "suspicion");

	const char *doing = "…";
	switch (target->activity) {
		case Activity::Scanning: doing = "looking around"; break;
		case Activity::Wandering: doing = "wandering"; break;
		case Activity::Investigating: doing = "investigating a noise"; break;
		case Activity::TakingCover: doing = "RUNNING FOR COVER"; break;
		case Activity::Engaging: doing = "SHOOTING AT YOU"; break;
		default: break;
	}
	get_node<Label>("%Doing")->set_text(String("he's ") + String::utf8(doing) + (s.sees_shooter ? String::utf8(" · sees you") : ""));
	get_node<Label>("%Help")->set_text(
			String("Up/Down move   Left/Right turn   Space fire   M target walks: ") + (game->target_mobile ? "on" : "off") +
			"   Tab radar: " + game->get_radar_mode());

	auto *banner = get_node<Control>("%Banner");
	banner->set_visible(game->state != GameState::Playing);
	auto *banner_text = get_node<Label>("%BannerText");
	if (game->state == GameState::Won) {
		banner_text->set_text("TARGET DOWN");
		banner_text->add_theme_color_override("font_color", Color::from_rgba8(90, 230, 110));
	} else if (game->state == GameState::Lost) {
		banner_text->set_text("YOU WERE SPOTTED. AND SHOT.");
		banner_text->add_theme_color_override("font_color", Color::from_rgba8(240, 70, 60));
	}
}

// ---------------------------------------------------------------------------
// Fx
// ---------------------------------------------------------------------------

void Fx::_ready() {
	Game *game = find_game(this);
	game->connect("gunshot", callable_mp(this, &Fx::on_gunshot));
	game->connect("bullet_impact", callable_mp(this, &Fx::on_impact));
	game->connect("target_hit", callable_mp(this, &Fx::on_target_hit));
	game->connect("shooter_hit", callable_mp(this, &Fx::on_shooter_hit));
}

void Fx::on_gunshot(Vector3 muzzle, Vector3 dir) {
	auto *light = memnew(OmniLight3D);
	light->set_color(Color(1.0f, 0.75f, 0.4f));
	light->set_param(Light3D::PARAM_ENERGY, 6.0f);
	light->set_param(Light3D::PARAM_RANGE, 14.0f);
	add_child(light);
	light->set_global_position(muzzle);
	expire(light, 0.08f);
	burst(muzzle, dir, 20.0f, 16, 6.0f, 14.0f, Color(1.0f, 0.6f, 0.2f), 0.18f);
}

void Fx::on_impact(Vector3 at) {
	burst(at, Vector3(0, 1, 0), 70.0f, 12, 2.0f, 6.0f, Color(1.0f, 0.85f, 0.5f), 0.35f);
}

void Fx::on_target_hit(Vector3 at) {
	burst(at, Vector3(0, 1, 0), 85.0f, 20, 1.0f, 4.0f, Color(0.8f, 0.0f, 0.0f), 0.5f);
}

void Fx::on_shooter_hit(Vector3 from, Vector3 to) {
	Ref<ImmediateMesh> line;
	line.instantiate();
	line->surface_begin(Mesh::PRIMITIVE_LINES);
	line->surface_add_vertex(from);
	line->surface_add_vertex(to);
	line->surface_end();
	auto *tracer = memnew(MeshInstance3D);
	tracer->set_mesh(line);
	tracer->set_material_override(unshaded(Color(1.0f, 0.3f, 0.2f)));
	add_child(tracer);
	expire(tracer, 0.12f);
}

void Fx::burst(Vector3 at, Vector3 dir, float spread_deg, int count, float min_speed, float max_speed, Color color,
		float lifetime) {
	auto *p = memnew(CPUParticles3D);
	Ref<SphereMesh> dot;
	dot.instantiate();
	dot->set_radius(0.05f);
	dot->set_height(0.1f);
	dot->set_radial_segments(4);
	dot->set_rings(2);
	Ref<StandardMaterial3D> mat = unshaded(color);
	mat->set_flag(BaseMaterial3D::FLAG_ALBEDO_FROM_VERTEX_COLOR, true);
	mat->set_transparency(BaseMaterial3D::TRANSPARENCY_ALPHA);
	mat->set_blend_mode(BaseMaterial3D::BLEND_MODE_ADD);
	dot->set_material(mat);
	p->set_mesh(dot);
	p->set_amount(count);
	p->set_one_shot(true);
	p->set_explosiveness_ratio(1.0f);
	p->set_lifetime(lifetime);
	p->set_direction(dir);
	p->set_spread(spread_deg);
	p->set_param_min(CPUParticles3D::PARAM_INITIAL_LINEAR_VELOCITY, min_speed);
	p->set_param_max(CPUParticles3D::PARAM_INITIAL_LINEAR_VELOCITY, max_speed);
	p->set_param_min(CPUParticles3D::PARAM_DAMPING, 2.0f);
	p->set_param_max(CPUParticles3D::PARAM_DAMPING, 2.0f);
	p->set_gravity(Vector3(0, -6, 0));
	Ref<Gradient> fade;
	fade.instantiate();
	fade->set_color(0, color);
	fade->set_color(1, Color(color, 0.0f));
	p->set_color_ramp(fade);
	p->set_use_local_coordinates(false);
	add_child(p);
	p->set_global_position(at);
	p->set_emitting(true);
	p->connect("finished", callable_mp((Node *)p, &Node::queue_free));
}

void Fx::expire(Node *node, float secs) {
	get_tree()->create_timer(secs)->connect("timeout", callable_mp(node, &Node::queue_free));
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

void Audio::_ready() {
	game = find_game(this);
	ResourceLoader *loader = ResourceLoader::get_singleton();
	shot = loader->load("res://sfx/shot.wav");
	impact = loader->load("res://sfx/impact.wav");
	target_hit = loader->load("res://sfx/target_hit.wav");
	return_fire = loader->load("res://sfx/return_fire.wav");
	step = loader->load("res://sfx/step.wav");
	// godot-cpp has no lambda callables, so each signal gets a small member function.
	game->connect("gunshot", callable_mp(this, &Audio::on_gunshot));
	game->connect("bullet_impact", callable_mp(this, &Audio::on_impact));
	game->connect("target_hit", callable_mp(this, &Audio::on_target_hit));
	game->connect("shooter_hit", callable_mp(this, &Audio::on_shooter_hit));
	game->connect("footstep", callable_mp(this, &Audio::on_footstep));
}

void Audio::on_gunshot(Vector3 at, Vector3) { play(shot, at, 80); }
void Audio::on_impact(Vector3 at) { play(impact, at, 30); }
void Audio::on_target_hit(Vector3 at) { play(target_hit, at, 10); }
void Audio::on_shooter_hit(Vector3 from, Vector3) { play(return_fire, from, 30); }
void Audio::on_footstep(Vector3 at) { play(step, at, 24); }

/** One-shot spatial sound, freed when it finishes. Inaudible beyond `radius`. */
void Audio::play(const Ref<AudioStream> &stream, Vector3 at, float radius) {
	if (game->state != GameState::Playing) {
		return;
	}
	auto *player = memnew(AudioStreamPlayer3D);
	player->set_stream(stream);
	player->set_max_distance(radius);
	player->set_unit_size(radius * 0.25f);
	add_child(player);
	player->set_global_position(at);
	player->connect("finished", callable_mp((Node *)player, &Node::queue_free));
	player->play();
}

} // namespace sp
