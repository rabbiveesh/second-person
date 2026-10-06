# Ports of the Bevy game's tests/gameplay.rs, against the real main.tscn and the C++ nodes.
# Run headless at a fixed 60 Hz:
#   godot --headless --path godot --fixed-fps 60 -s res://tests/gameplay.gd
# Each test gets a fresh main scene; input is driven with Input.action_press.
extends SceneTree

const MAIN := preload("res://scenes/main.tscn")
const TARGET_MAX_HP := 3
const SHOOTER_MAX_HP := 100.0
const MIN_START_DISTANCE := 12.0

var game: Node
var failures: Array[String] = []
var current := ""


func _initialize() -> void:
	_run_all.call_deferred()


func _run_all() -> void:
	var tests := [
		"round_starts_with_one_of_each",
		"firing_spawns_a_bullet_and_a_gunshot",
		"the_shooter_drives_forward_and_turns",
		"walls_stop_the_shooter",
		"shooting_the_target_hurts_and_alerts_him",
		"he_sees_the_shooter_in_his_view",
		"he_doesnt_see_the_shooter_behind_him",
		"cover_blocks_line_of_sight",
		"nearby_gunshot_makes_him_investigate",
		"engaged_target_runs_for_cover",
		"engaged_target_eventually_kills_an_exposed_shooter_then_r_restarts",
		"navmesh_routes_around_cover",
		"the_target_walks_around_when_mobile",
		"radar_cant_see_actors_directly",
		"cover_spots_hide_from_the_threat",
		"random_starts_are_clear_of_cover_and_the_target",
	]
	var only := OS.get_environment("ONLY")
	var passed := 0
	for t in tests:
		if only != "" and not t.contains(only):
			continue
		current = t
		var before := failures.size()
		await _setup()
		await call(t)
		await _teardown()
		if failures.size() == before:
			passed += 1
			print("ok   ", t)
		else:
			print("FAIL ", t)
	for f in failures:
		printerr(f)
	print("%d passed, %d failed" % [passed, failures.size()])
	quit(1 if failures.size() > 0 else 0)


func _setup() -> void:
	for a in ["forward", "back", "turn_left", "turn_right", "fire", "restart", "toggle_mobility", "cycle_radar"]:
		Input.action_release(a)
	game = MAIN.instantiate()
	game.set_seed(1)
	root.add_child(game)
	await tick()


func _teardown() -> void:
	game.queue_free()
	await process_frame


# --- helpers -----------------------------------------------------------------------------

func expect(cond: bool, what: String) -> void:
	if not cond:
		failures.append("%s: %s" % [current, what])


func tick(n := 1) -> void:
	for i in n:
		await physics_frame


func run(secs: float) -> void:
	await tick(ceili(secs * 60.0))


func press(action: String) -> void:
	Input.action_press(action)
	await tick()
	Input.action_release(action)
	await tick()


func shooter() -> Node3D:
	return game.get_shooter()


func target() -> Node3D:
	return game.get_target()


func arena() -> Node:
	return get_first_node_in_group("arena")


func counter(signal_name: String) -> Array:
	var c := [0]
	game.connect(signal_name, func(_a = null, _b = null): c[0] += 1)
	return c


func xz(v: Vector3) -> Vector2:
	return Vector2(v.x, v.z)


## Yaw that makes `from` face `to` (forward is -Z).
func yaw_towards(from: Vector3, to: Vector3) -> float:
	var d := to - from
	return atan2(-d.x, -d.z)


## Target at the origin facing `faces`, shooter at `shooter_pos` facing the target.
func stage(shooter_pos: Vector3, faces: Vector3) -> void:
	var t := Vector3(0, 0.9, 0)
	target().place(t, yaw_towards(t, faces))
	shooter().place(shooter_pos, yaw_towards(shooter_pos, t))
	await tick()


# --- gameplay ----------------------------------------------------------------------------

func round_starts_with_one_of_each() -> void:
	expect(game.get_state() == "Playing", "state is %s" % game.get_state())
	expect(shooter().hp == SHOOTER_MAX_HP, "shooter hp")
	expect(target().hp == TARGET_MAX_HP, "target hp")
	expect(root.get_viewport().get_camera_3d() == target().get_node("Head/Eyes"), "you see through his eyes")


func firing_spawns_a_bullet_and_a_gunshot() -> void:
	var shots := counter("gunshot")
	await press("fire")
	expect(shots[0] == 1, "gunshots: %d" % shots[0])
	expect(game.bullet_count() == 1, "bullets: %d" % game.bullet_count())


func the_shooter_drives_forward_and_turns() -> void:
	var start := Vector3(0, 0.9, 12)
	shooter().place(start, 0.0) # facing -Z, toward the origin
	await tick()
	Input.action_press("forward")
	await run(0.5)
	Input.action_release("forward")
	var moved := shooter().global_position - start
	expect(moved.z < -1.5, "moved %s" % moved)
	expect(absf(moved.x) < 0.1, "drifted %s" % moved)
	expect(absf(shooter().global_position.y - 0.9) < 0.2, "height %f" % shooter().global_position.y)

	Input.action_press("turn_left")
	await run(0.5)
	expect(shooter().rotation.y > 1.0, "yaw %f" % shooter().rotation.y)


func walls_stop_the_shooter() -> void:
	shooter().place(Vector3(0, 0.9, 25), PI) # facing +Z, toward the wall at z=30
	await tick()
	Input.action_press("forward")
	await run(3.0)
	expect(shooter().global_position.z < 29.5 - 0.3, "z %f" % shooter().global_position.z)


func shooting_the_target_hurts_and_alerts_him() -> void:
	var hits := counter("target_hit")
	# Shooter right behind him: point blank, and out of his view.
	await stage(Vector3(0, 0.9, 6), Vector3(0, 0.9, -10))
	await press("fire")
	await run(0.5)
	expect(hits[0] == 1, "hits: %d" % hits[0])
	expect(target().hp == TARGET_MAX_HP - 1, "hp %d" % target().hp)
	expect(target().get_suspicion() > 0.5, "suspicion %f" % target().get_suspicion())


func he_sees_the_shooter_in_his_view() -> void:
	var p := Vector3(0, 0.9, -10)
	await stage(p, p)
	await tick()
	expect(target().get_sees_shooter(), "doesn't see him")
	expect(target().get_suspicion() > 0.0, "no suspicion")


func he_doesnt_see_the_shooter_behind_him() -> void:
	await stage(Vector3(0, 0.9, 10), Vector3(0, 0.9, -10))
	await tick()
	expect(not target().get_sees_shooter(), "sees behind himself")


func cover_blocks_line_of_sight() -> void:
	# Crate at (-6, _, -8); shooter directly behind it as seen from the origin.
	var p := Vector3(-9, 0.9, -12)
	await stage(p, p)
	await tick()
	expect(not target().get_sees_shooter(), "sees through the crate")


func nearby_gunshot_makes_him_investigate() -> void:
	# Behind him (out of view) but within hearing range, firing away from him.
	await stage(Vector3(0, 0.9, 10), Vector3(0, 0.9, -10))
	shooter().place(Vector3(0, 0.9, 10), PI)
	await press("fire")
	await run(0.2)
	expect(target().has_alert(), "not alerted")
	expect(target().get_activity() == "Investigating", "activity %s" % target().get_activity())


func engaged_target_runs_for_cover() -> void:
	var p := Vector3(0, 0.9, -12)
	await stage(p, p)
	target().engage(p)
	var took_cover := false
	var hidden := false
	for i in 6 * 60:
		await tick()
		took_cover = took_cover or target().get_activity() == "TakingCover"
		hidden = hidden or arena().los_blocked(xz(p), xz(target().global_position))
	expect(took_cover, "never ran for cover")
	expect(hidden, "never got out of the shooter's line of sight")


func engaged_target_eventually_kills_an_exposed_shooter_then_r_restarts() -> void:
	var hurt := counter("shooter_hit")
	var p := Vector3(0, 0.9, -8)
	await stage(p, p)
	target().engage(p)
	# He hides and peeks, so this takes a while; a shooter standing in the open still loses.
	var trace := OS.get_environment("TRACE") != ""
	var i := 0
	while i < 90 * 60 and game.get_state() == "Playing":
		await tick()
		if trace and i % 30 == 0:
			var t := target()
			print("t=%.1f %s pos=%s lvl=%.2f eng=%s sees=%s" % [i / 60.0, t.get_activity(), xz(t.global_position),
				t.get_suspicion(), t.get_engaged(), t.get_sees_shooter()])
		i += 1
	expect(hurt[0] > 0, "never shot back")
	expect(game.get_state() == "Lost", "state %s" % game.get_state())

	var old_camera: Camera3D = target().get_node("Head/Eyes")
	await press("restart")
	expect(game.get_state() == "Playing", "restart didn't restart")
	expect(shooter().hp == SHOOTER_MAX_HP, "shooter hp not reset")
	expect(not is_instance_valid(old_camera), "old camera still alive")
	expect(root.get_viewport().get_camera_3d() == target().get_node("Head/Eyes"), "camera not the new target's")


func navmesh_routes_around_cover() -> void:
	# Straight line from (-10,-6) to (-10,2) goes through the pillar at (-10,-2).
	var a := Vector3(-10, 0.9, -6)
	var b := Vector3(-10, 0.9, 2)
	var path: PackedVector3Array = arena().path(a, b)
	expect(path.size() > 1, "path %s" % path)
	if path.is_empty():
		return
	expect(xz(path[-1]).distance_to(xz(b)) < 0.5, "ends at %s" % path[-1])
	var prev := xz(a)
	for w in path:
		for i in 21:
			var q := prev.lerp(xz(w), i / 20.0)
			if not arena().is_clear(q, 0.3):
				expect(false, "path passes through cover at %s" % q)
				return
		prev = xz(w)


func the_target_walks_around_when_mobile() -> void:
	await press("toggle_mobility")
	expect(game.get_target_mobile(), "M didn't toggle")
	shooter().place(Vector3(27, 0.9, 27), 0.0) # tucked in a corner, out of the way
	var start := target().global_position
	await run(3.0)
	expect(target().get_activity() == "Wandering", "activity %s" % target().get_activity())
	expect(start.distance_to(target().global_position) > 2.0, "moved %f" % start.distance_to(target().global_position))


func radar_cant_see_actors_directly() -> void:
	var radar_world: Node = game.find_child("World", true, false)
	for n in game.find_children("*", "VisualInstance3D", true, false):
		if n.is_in_group("live_blip") or (radar_world and radar_world.is_ancestor_of(n)):
			continue
		expect(n.layers & 2 == 0, "%s is visible on radar" % n.get_path())


# --- arena geometry ----------------------------------------------------------------------

func cover_spots_hide_from_the_threat() -> void:
	for threat in [Vector2(0, -12), Vector2(15, 15), Vector2(-20, 0)]:
		var cover: Array = arena().find_cover(Vector2.ZERO, threat)
		expect(cover.size() == 2, "no cover from %s" % threat)
		if cover.is_empty():
			continue
		expect(arena().los_blocked(threat, cover[0]), "%s visible from %s" % [cover[0], threat])
		expect(arena().is_clear(cover[0], 0.5), "%s inside cover" % cover[0])


func random_starts_are_clear_of_cover_and_the_target() -> void:
	var rng := RandomNumberGenerator.new()
	rng.seed = 42
	for i in 500:
		var s: Vector3 = Shooter.sample_start(rng)
		var p := Vector2(s.x, s.y)
		if p.length() < MIN_START_DISTANCE or not arena().is_clear(p, 0.35):
			expect(false, "bad start %s" % p)
			return
