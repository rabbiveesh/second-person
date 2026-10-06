# Plays a few seconds with a real renderer and saves screenshots (needs a display, e.g. Xvfb):
#   godot --path godot --rendering-driver opengl3 --fixed-fps 60 -s res://tests/smoke.gd -- <out dir>
extends SceneTree

var game: Node


func _initialize() -> void:
	_run.call_deferred()


func _run() -> void:
	var args := OS.get_cmdline_user_args()
	var out: String = args[0] if args.size() > 0 else "user://"
	game = load("res://scenes/main.tscn").instantiate()
	root.add_child(game)
	await frames(30)
	# Put the shooter in his view, a little off-centre, then fire a couple of times.
	game.get_target().place(Vector3(0, 0.9, 0), 0.0)
	game.get_shooter().place(Vector3(3, 0.9, -14), atan2(3.0, -14.0))
	await frames(20)
	await press("fire")
	await frames(3)
	await shot(out + "/godot-view.png")
	await frames(60)
	await press("cycle_radar") # Sonar -> Off
	await press("cycle_radar") # Off -> Full
	await frames(10)
	await shot(out + "/godot-full-radar.png")
	game.get_target().engage(game.get_shooter().global_position)
	await frames(240)
	await shot(out + "/godot-engaged.png")
	quit()


func frames(n: int) -> void:
	for i in n:
		await process_frame


func press(action: String) -> void:
	Input.action_press(action)
	await physics_frame
	await process_frame
	Input.action_release(action)
	await process_frame


func shot(path: String) -> void:
	await RenderingServer.frame_post_draw
	root.get_texture().get_image().save_png(path)
	print("saved ", path)
