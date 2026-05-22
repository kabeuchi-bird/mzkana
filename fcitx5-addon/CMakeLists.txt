cmake_minimum_required(VERSION 3.20)
project(fcitx5-mzkana VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# ── Paths ─────────────────────────────────────────────────────────────────────

# Rust release build directory (contains libmzkana.so)
set(MZKANA_RUST_TARGET "${CMAKE_SOURCE_DIR}/../target/release"
    CACHE PATH "Rust target/release directory")

# Generated C header (produced by build.rs / cbindgen)
set(MZKANA_FFI_INCLUDE "${CMAKE_SOURCE_DIR}/../crates/mzkana-ffi/include"
    CACHE PATH "Directory containing mzkana.h")

# fcitx5 source headers (sparse checkout from GitHub)
set(FCITX5_SRC_INCLUDE "/tmp/fcitx5-src/src/lib"
    CACHE PATH "fcitx5 src/lib directory with fcitx/ fcitx-utils/ fcitx-config/")

# Stub generated headers (fcitxcore_export.h etc.) supplied in this tree
set(FCITX5_STUB_INCLUDE "${CMAKE_SOURCE_DIR}/cmake/include"
    CACHE PATH "Stub export headers for fcitx5")

# Installed fcitx5 shared libraries (from libfcitx5core7 / libfcitx5utils2)
set(FCITX5_CORE_LIB "/usr/lib/x86_64-linux-gnu/libFcitx5Core.so"
    CACHE FILEPATH "libFcitx5Core shared library")
set(FCITX5_UTILS_LIB "/usr/lib/x86_64-linux-gnu/libFcitx5Utils.so"
    CACHE FILEPATH "libFcitx5Utils shared library")

# Install destinations
set(FCITX5_ADDON_DIR "/usr/lib/x86_64-linux-gnu/fcitx5"
    CACHE PATH "fcitx5 addon install directory")
set(FCITX5_DATA_DIR "/usr/share/fcitx5"
    CACHE PATH "fcitx5 data install directory")

# ── Imported targets ──────────────────────────────────────────────────────────

add_library(Mzkana::FFI SHARED IMPORTED)
set_target_properties(Mzkana::FFI PROPERTIES
    IMPORTED_LOCATION "${MZKANA_RUST_TARGET}/libmzkana.so"
    INTERFACE_INCLUDE_DIRECTORIES "${MZKANA_FFI_INCLUDE}"
)

add_library(Fcitx5::Core SHARED IMPORTED)
set_target_properties(Fcitx5::Core PROPERTIES
    IMPORTED_LOCATION "${FCITX5_CORE_LIB}"
    INTERFACE_INCLUDE_DIRECTORIES "${FCITX5_SRC_INCLUDE};${FCITX5_STUB_INCLUDE}"
)

add_library(Fcitx5::Utils SHARED IMPORTED)
set_target_properties(Fcitx5::Utils PROPERTIES
    IMPORTED_LOCATION "${FCITX5_UTILS_LIB}"
    INTERFACE_INCLUDE_DIRECTORIES "${FCITX5_SRC_INCLUDE};${FCITX5_STUB_INCLUDE}"
)

# ── Addon module ──────────────────────────────────────────────────────────────

add_library(fcitx5-mzkana MODULE
    src/engine.cpp
)

target_include_directories(fcitx5-mzkana PRIVATE
    ${MZKANA_FFI_INCLUDE}
    ${FCITX5_SRC_INCLUDE}
    ${FCITX5_STUB_INCLUDE}
)

target_compile_options(fcitx5-mzkana PRIVATE
    -fvisibility=hidden
    -Wall -Wextra
)

target_link_libraries(fcitx5-mzkana PRIVATE
    Mzkana::FFI
    Fcitx5::Core
    Fcitx5::Utils
)

# Fcitx5 addon .so must be named without the "lib" prefix
set_target_properties(fcitx5-mzkana PROPERTIES
    PREFIX ""
)

# ── Install ───────────────────────────────────────────────────────────────────

include(GNUInstallDirs)

install(TARGETS fcitx5-mzkana
    DESTINATION "${FCITX5_ADDON_DIR}")

install(FILES data/mzkana.conf
    DESTINATION "${FCITX5_DATA_DIR}/addon")

install(FILES data/mzkana.desktop
    DESTINATION "${FCITX5_DATA_DIR}/inputmethod")

install(FILES "${MZKANA_RUST_TARGET}/libmzkana.so"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}")
