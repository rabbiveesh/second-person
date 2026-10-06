#include "target.h"

#include "layers.h"
#include "shooter.h"

#include <behaviortree_cpp/bt_factory.h>

#include <godot_cpp/classes/array_mesh.hpp>
#include <godot_cpp/classes/mesh_instance3d.hpp>
#include <godot_cpp/classes/physics_direct_space_state3d.hpp>
#include <godot_cpp/classes/physics_ray_query_parameters3d.hpp>
#include <godot_cpp/classes/scene_tree.hpp>
#include <godot_cpp/classes/standard_material3d.hpp>
#include <godot_cpp/classes/world3d.hpp>
#include <godot_cpp/core/class_db.hpp>

#include <cmath>
#include <functional>

using namespace godot;

namespace sp {

static constexpr float BODY_CENTER = 0.9f;
static constexpr float EYE_OFFSET = 0.7f;
static constexpr float WALK_SPEED = 2.2f;
static constexpr float RUN_SPEED = 5.5f;
/** "Close enough" for every arrival check (walk itself homes in to 0.2m). */
static constexpr float ARRIVE = 0.5f;

// The tree, in BehaviorTree.CPP's XML. ReactiveFallback ticks its children in order every
// tick, so a higher branch whose condition turns true halts whatever lower branch was running.
static const char *TREE_XML = R"(
<root BTCPP_format="4">
  <BehaviorTree ID="Target">
    <ReactiveFallback>
      <ReactiveSequence>
        <IsEngaged/>
        <Sequence>
          <TakeCover/>
          <Fight/>
        </Sequence>
      </ReactiveSequence>
      <ReactiveSequence>
        <IsAlerted/>
        <Investigate/>
      </ReactiveSequence>
      <ReactiveSequence>
        <IsMobile/>
        <Wander/>
      </ReactiveSequence>
      <Scan/>
    </ReactiveFallback>
  </BehaviorTree>
</root>)";

/** A BT action made of plain callbacks: start (first tick), running (later ticks), halted. */
class Act : public BT::StatefulActionNode {
public:
	using Tick = std::function<BT::NodeStatus()>;
	Act(const std::string &name, const BT::NodeConfig &cfg, Tick start, Tick running, std::function<void()> halted) :
			BT::StatefulActionNode(name, cfg), start(std::move(start)), running(std::move(running)), halted(std::move(halted)) {}
	static BT::PortsList providedPorts() { return {}; }
	BT::NodeStatus onStart() override { return start(); }
	BT::NodeStatus onRunning() override { return running(); }
	void onHalted() override { halted(); }

private:
	Tick start, running;
	std::function<void()> halted;
};

using BT::NodeStatus;

/** Builds the tree and wires each node to the target's state. */
struct TargetTree {
	static std::unique_ptr<BT::Tree> build(Target *t) {
		BT::BehaviorTreeFactory f;
		auto &s = t->suspicion;
		auto here = [t] { return xz(t->get_global_position()); };
		auto go_to = [t](Vector2 p, float speed, bool strafe) {
			t->set_move_to(MoveTo{ on_ground(p, BODY_CENTER), speed, strafe });
		};
		auto stop = [t] { t->set_move_to(std::nullopt); };
		auto action = [&f](const char *id, Act::Tick start, Act::Tick running, std::function<void()> halted) {
			f.registerBuilder<Act>(id, [=](const std::string &name, const BT::NodeConfig &cfg) {
				return std::make_unique<Act>(name, cfg, start, running, halted);
			});
		};
		auto cond = [&f](const char *id, std::function<bool()> c) {
			f.registerSimpleCondition(id, [c](BT::TreeNode &) { return c() ? NodeStatus::SUCCESS : NodeStatus::FAILURE; });
		};

		cond("IsEngaged", [&s] { return s.engaged; });
		cond("IsAlerted", [t] { return t->alert.has_value(); });
		cond("IsMobile", [t] { return t->mobile; });

		// Run to the nearest spot hidden from where he last saw or heard the shooter.
		auto take_cover = [=] {
			t->activity = Activity::TakingCover;
			if (here().distance_to(t->cover_plan.spot) >= ARRIVE) {
				return NodeStatus::RUNNING;
			}
			stop();
			return NodeStatus::SUCCESS;
		};
		action(
				"TakeCover",
				[=, &s] {
					const Vector2 threat = s.last_known ? xz(*s.last_known) : here() + xz(t->forward_flat()) * 10.0f;
					t->cover_plan = find_cover(here(), threat).value_or(Cover{ here(), here() });
					go_to(t->cover_plan.spot, RUN_SPEED, false);
					return take_cover();
				},
				take_cover, stop);

		// Hide behind cover, then peek out to shoot, then hide again. Succeeds (so the tree
		// re-plans cover) when he's no longer engaged or gets hit.
		auto fight = [=, &s] {
			t->activity = Activity::Engaging;
			auto &f = t->fighting;
			const Cover &plan = t->cover_plan;
			if (!s.engaged || t->hp < f.hp_at_start) {
				stop();
				return NodeStatus::SUCCESS;
			}
			if (s.last_known) {
				t->look_point = *s.last_known + Vector3(0, EYE_OFFSET, 0);
				t->turn_speed = 4.0f;
			}
			const Vector2 goal = f.peeking ? plan.peek : plan.spot;
			if (here().distance_to(goal) >= ARRIVE) {
				if (!t->move_to) {
					go_to(goal, RUN_SPEED, true);
				}
				return NodeStatus::RUNNING;
			}
			f.timer -= t->dt;
			if (f.timer <= 0.0f) {
				f.peeking = !f.peeking;
				f.timer = f.peeking ? 1.8f : t->rng->randf_range(0.8f, 2.0f);
				go_to(f.peeking ? plan.peek : plan.spot, RUN_SPEED, true);
			}
			return NodeStatus::RUNNING;
		};
		action(
				"Fight",
				[=] {
					t->fighting = { false, t->rng->randf_range(0.8f, 1.6f), t->hp };
					return fight();
				},
				fight, stop);

		auto investigate = [=] {
			t->activity = Activity::Investigating;
			auto &a = *t->alert;
			t->look_point = a.at;
			t->turn_speed = 3.5f;
			a.remaining -= t->dt;
			if (a.remaining <= 0.0f) {
				t->alert.reset();
				return NodeStatus::SUCCESS;
			}
			return NodeStatus::RUNNING;
		};
		action("Investigate", investigate, investigate, [] {});

		auto wander = [=] {
			t->activity = Activity::Wandering;
			if (!t->move_to || here().distance_to(xz(t->move_to->dest)) < ARRIVE) {
				stop();
				return NodeStatus::SUCCESS;
			}
			return NodeStatus::RUNNING;
		};
		action(
				"Wander",
				[=] {
					const float r = ARENA_HALF - 4.0f;
					go_to(Vector2(t->rng->randf_range(-r, r), t->rng->randf_range(-r, r)), WALK_SPEED, false);
					return wander();
				},
				wander, stop);

		auto scan = [=] {
			t->activity = Activity::Scanning;
			auto &plan = t->scan_plan;
			const Vector3 eye = t->get_global_position() + Vector3(0, EYE_OFFSET, 0);
			t->look_point = eye + Vector3(-std::sin(plan.yaw), 0, -std::cos(plan.yaw)) * 10.0f;
			t->turn_speed = 1.2f;
			if (std::abs(angle_diff(t->yaw, plan.yaw)) < 0.05f) {
				plan.dwell -= t->dt;
				if (plan.dwell <= 0.0f) {
					return NodeStatus::SUCCESS;
				}
			}
			return NodeStatus::RUNNING;
		};
		action(
				"Scan",
				[=] {
					const float swing = t->rng->randf_range(0.6f, 2.6f) * (t->rng->randf() < 0.5f ? 1.0f : -1.0f);
					t->scan_plan = { t->yaw + swing, t->rng->randf_range(0.6f, 2.2f) };
					return scan();
				},
				scan, [] {});

		return std::make_unique<BT::Tree>(f.createTreeFromText(TREE_XML));
	}
};

Target::Target() {
	rng.instantiate();
}

Target::~Target() = default;

void Target::_ready() {
	head = get_node<Node3D>("Head");
	camera = get_node<Camera3D>("Head/Eyes");
	yaw = get_global_rotation().y;
	look_point = get_global_position() + Vector3(-std::sin(yaw), EYE_OFFSET, -std::cos(yaw)) * 10.0f;
	tree = TargetTree::build(this);
	build_view_cone();
}

/** The radar's view-cone sector: a flat triangle fan, built here because .tscn can't express it. */
void Target::build_view_cone() {
	auto *cone = get_node<MeshInstance3D>("ViewCone");
	PackedVector3Array verts;
	const int segments = 16;
	for (int i = 0; i < segments; i++) {
		const float a0 = -VIEW_HALF_ANGLE + 2.0f * VIEW_HALF_ANGLE * i / segments;
		const float a1 = -VIEW_HALF_ANGLE + 2.0f * VIEW_HALF_ANGLE * (i + 1) / segments;
		verts.push_back(Vector3());
		verts.push_back(Vector3(-std::sin(a1), 0.0f, -std::cos(a1)) * VIEW_RANGE * 0.5f);
		verts.push_back(Vector3(-std::sin(a0), 0.0f, -std::cos(a0)) * VIEW_RANGE * 0.5f);
	}
	Array arrays;
	arrays.resize(Mesh::ARRAY_MAX);
	arrays[Mesh::ARRAY_VERTEX] = verts;
	Ref<ArrayMesh> mesh;
	mesh.instantiate();
	mesh->add_surface_from_arrays(Mesh::PRIMITIVE_TRIANGLES, arrays);
	Ref<StandardMaterial3D> mat;
	mat.instantiate();
	mat->set_shading_mode(BaseMaterial3D::SHADING_MODE_UNSHADED);
	mat->set_transparency(BaseMaterial3D::TRANSPARENCY_ALPHA);
	mat->set_cull_mode(BaseMaterial3D::CULL_DISABLED);
	mat->set_albedo(Color(1.0f, 0.85f, 0.3f, 0.22f));
	mesh->surface_set_material(0, mat);
	cone->set_mesh(mesh);
}

void Target::set_move_to(std::optional<MoveTo> m) {
	move_to = m;
	route.clear();
	if (m && arena) {
		// Plan immediately; fall back to a straight line if the navmesh has no path.
		route = arena->path(get_global_position(), m->dest);
	}
	if (m && route.is_empty()) {
		route.push_back(m->dest);
	}
}

void Target::_physics_process(double delta) {
	dt = float(delta);
	elapsed += dt;
	if (!shooter) {
		shooter = Object::cast_to<Shooter>(get_tree()->get_first_node_in_group("shooter"));
		arena = Object::cast_to<Arena>(get_tree()->get_first_node_in_group("arena"));
		if (!shooter) {
			return;
		}
	}
	perceive(dt);
	tree->tickOnce();
	gaze(dt);
	walk(dt);
}

/** Vision: is the shooter inside the view cone with clear line of sight? Feeds `suspicion`. */
void Target::perceive(float dt) {
	Suspicion &s = suspicion;
	const Vector3 eye = camera->get_global_position();
	const Vector3 to_shooter = shooter->get_global_position() - eye;
	const float dist = to_shooter.length();
	const float angle = eye_forward().angle_to(to_shooter);

	bool sees = dist < VIEW_RANGE && angle < VIEW_HALF_ANGLE;
	if (sees) {
		auto query = PhysicsRayQueryParameters3D::create(eye, shooter->get_global_position(), Phys::World | Phys::Shooter);
		Dictionary hit = get_world_3d()->get_direct_space_state()->intersect_ray(query);
		sees = !hit.is_empty() && Object::cast_to<Object>(hit["collider"]) == shooter;
	}
	s.sees_shooter = sees;
	if (sees) {
		s.last_known = shooter->get_global_position();
		const float closeness = 1.0f - dist / VIEW_RANGE;
		const float centred = 1.0f - angle / VIEW_HALF_ANGLE;
		const float moving = shooter->get_speed() > 0.5f ? 1.0f : 0.25f;
		s.bump((0.08f + 0.6f * closeness) * (0.4f + 0.6f * centred) * moving * dt);
		// Half-sure: glance over.
		if (s.level > 0.5f && !s.engaged) {
			raise_alert(shooter->get_global_position(), 1.5f);
		}
	} else {
		s.level = std::max(s.level - 0.12f * dt, 0.0f);
		if (s.level <= 0.0f) {
			s.engaged = false;
		}
	}
}

/** Turn the body (yaw) and head (pitch) toward the look goal. */
void Target::gaze(float dt) {
	const Vector3 eye = get_global_position() + Vector3(0, EYE_OFFSET, 0);
	const Vector3 d = look_point - eye;
	const float step = turn_speed * dt;

	const float desired_yaw = std::atan2(-d.x, -d.z);
	yaw += CLAMP(angle_diff(yaw, desired_yaw), -step, step);
	set_global_rotation(Vector3(0, yaw, 0));

	// Pitch, plus a little idle sway so the view feels alive.
	const float desired_pitch = CLAMP(std::atan2(d.y, Vector2(d.x, d.z).length()), -0.6f, 0.6f) + 0.03f * std::sin(elapsed * 0.7f);
	pitch += CLAMP(desired_pitch - pitch, -step, step);
	head->set_rotation(Vector3(pitch, 0, 0));
}

/** Follow the route planned for `move_to`, looking where he's going; turn before moving. */
void Target::walk(float) {
	set_velocity(Vector3());
	if (!move_to) {
		return;
	}
	const Vector2 here = xz(get_global_position());
	while (route.size() > 1 && xz(route[0]).distance_to(here) < 0.35f) {
		route.remove_at(0);
	}
	const Vector2 w = xz(route.is_empty() ? move_to->dest : route[0]);
	const Vector2 d = w - here;
	if (d.length() < 0.2f) {
		return;
	}
	const Vector3 dir = Vector3(d.x, 0, d.y).normalized();
	if (!move_to->strafe) {
		look_point = Vector3(w.x, get_global_position().y + EYE_OFFSET, w.y);
		turn_speed = 7.0f;
		if (forward_flat().angle_to(dir) >= 0.5f) {
			return;
		}
	}
	set_velocity(dir * move_to->speed);
	move_and_slide();
}

Vector3 Target::forward_flat() const {
	return Vector3(-std::sin(yaw), 0, -std::cos(yaw));
}

String Target::get_activity() const {
	switch (activity) {
		case Activity::Scanning:
			return "Scanning";
		case Activity::Wandering:
			return "Wandering";
		case Activity::Investigating:
			return "Investigating";
		case Activity::TakingCover:
			return "TakingCover";
		case Activity::Engaging:
			return "Engaging";
		default:
			return "";
	}
}

void Target::engage(Vector3 last_known) {
	suspicion.bump(1.0f);
	suspicion.last_known = last_known;
}

void Target::place(Vector3 pos, float new_yaw) {
	set_global_position(pos);
	yaw = new_yaw;
	set_global_rotation(Vector3(0, yaw, 0));
	look_point = pos + Vector3(-std::sin(yaw), EYE_OFFSET, -std::cos(yaw)) * 10.0f;
	pitch = 0.0f;
	head->set_rotation(Vector3());
}

float angle_diff(float a, float b) {
	const float tau = float(Math::TAU);
	return std::fmod(std::fmod(b - a + float(Math::PI), tau) + tau, tau) - float(Math::PI);
}

void Target::_bind_methods() {
	ClassDB::bind_method(D_METHOD("get_hp"), &Target::get_hp);
	ClassDB::bind_method(D_METHOD("set_hp", "hp"), &Target::set_hp);
	ADD_PROPERTY(PropertyInfo(Variant::INT, "hp"), "set_hp", "get_hp");
	ClassDB::bind_method(D_METHOD("get_suspicion"), &Target::get_suspicion);
	ClassDB::bind_method(D_METHOD("get_sees_shooter"), &Target::get_sees_shooter);
	ClassDB::bind_method(D_METHOD("get_engaged"), &Target::get_engaged);
	ClassDB::bind_method(D_METHOD("get_activity"), &Target::get_activity);
	ClassDB::bind_method(D_METHOD("has_alert"), &Target::has_alert);
	ClassDB::bind_method(D_METHOD("engage", "last_known"), &Target::engage);
	ClassDB::bind_method(D_METHOD("place", "pos", "yaw"), &Target::place);
}

} // namespace sp
