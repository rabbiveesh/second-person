# Native game code: a GDExtension shared library that Godot loads (see second_person.gdextension).
#   cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release && cmake --build build
cmake_minimum_required(VERSION 3.17)
project(second_person LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

# godot-cpp: C++ bindings to the engine, targeting the Godot 4.6 API.
set(GODOTCPP_API_VERSION 4.6 CACHE STRING "" FORCE)
set(GODOTCPP_TARGET template_debug CACHE STRING "" FORCE)
add_subdirectory(thirdparty/godot-cpp)
# Keep Godot from importing the build tree as project assets.
file(WRITE ${CMAKE_BINARY_DIR}/.gdignore "")

# BehaviorTree.CPP: the target's AI. Only the core library; no Groot/ZeroMQ, SQLite, tools or tests.
set(BTCPP_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(BTCPP_BUILD_TOOLS OFF CACHE BOOL "" FORCE)
set(BTCPP_EXAMPLES OFF CACHE BOOL "" FORCE)
set(BUILD_TESTING OFF CACHE BOOL "" FORCE)
set(BTCPP_GROOT_INTERFACE OFF CACHE BOOL "" FORCE)
set(BTCPP_SQLITE_LOGGING OFF CACHE BOOL "" FORCE)
add_subdirectory(thirdparty/BehaviorTree.CPP EXCLUDE_FROM_ALL)

file(GLOB SOURCES CONFIGURE_DEPENDS native/src/*.cpp)
add_library(second_person SHARED ${SOURCES})
target_link_libraries(second_person PRIVATE godot::cpp behaviortree_cpp)
# Godot loads it from bin/ (see second_person.gdextension).
set_target_properties(second_person PROPERTIES
    LIBRARY_OUTPUT_DIRECTORY ${CMAKE_SOURCE_DIR}/bin
    RUNTIME_OUTPUT_DIRECTORY ${CMAKE_SOURCE_DIR}/bin
    OUTPUT_NAME second_person)
