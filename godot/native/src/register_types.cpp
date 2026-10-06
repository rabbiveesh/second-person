// GDExtension entry point: registers the game's C++ classes so scenes can use them as node types.
#include "arena.h"
#include "game.h"
#include "presentation.h"
#include "shooter.h"
#include "target.h"

#include <gdextension_interface.h>
#include <godot_cpp/core/class_db.hpp>
#include <godot_cpp/godot.hpp>

using namespace godot;

static void initialize(ModuleInitializationLevel level) {
	if (level != MODULE_INITIALIZATION_LEVEL_SCENE) {
		return;
	}
	GDREGISTER_CLASS(sp::Arena);
	GDREGISTER_CLASS(sp::Shooter);
	GDREGISTER_CLASS(sp::Target);
	GDREGISTER_CLASS(sp::Game);
	GDREGISTER_CLASS(sp::Radar);
	GDREGISTER_CLASS(sp::Hud);
	GDREGISTER_CLASS(sp::Fx);
	GDREGISTER_CLASS(sp::Audio);
}

static void uninitialize(ModuleInitializationLevel) {}

extern "C" GDExtensionBool GDE_EXPORT second_person_init(GDExtensionInterfaceGetProcAddress get_proc_address,
		GDExtensionClassLibraryPtr library, GDExtensionInitialization *init) {
	GDExtensionBinding::InitObject init_obj(get_proc_address, library, init);
	init_obj.register_initializer(initialize);
	init_obj.register_terminator(uninitialize);
	init_obj.set_minimum_library_initialization_level(MODULE_INITIALIZATION_LEVEL_SCENE);
	return init_obj.init();
}
