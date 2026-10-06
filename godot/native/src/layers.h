// Physics and render layer bits. Names and the same numbering are also set in project.godot
// ([layer_names]), so the editor's layer pickers show them.
#pragma once

#include <cstdint>

namespace sp {

namespace Phys {
constexpr uint32_t World = 1 << 0;
constexpr uint32_t Shooter = 1 << 1;
constexpr uint32_t Target = 1 << 2;
constexpr uint32_t Bullet = 1 << 3;
} // namespace Phys

namespace Render {
/** Main view. Godot meshes default to layer 1 only, so this is opt-out-safe like Bevy. */
constexpr uint32_t World = 1 << 0;
/** Radar-only: the flat map, blips and sonar contacts. */
constexpr uint32_t Radar = 1 << 1;
} // namespace Render

} // namespace sp
