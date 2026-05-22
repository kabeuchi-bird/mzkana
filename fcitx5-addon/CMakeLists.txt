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

# Resolve platform-correct install paths early so FCITX5_ADDON_DIR/FCITX5_DATA_DIR
# can use them as defaults.
include(GNUInstallDirs)

# Install destinations – defaults derived from GNUInstallDirs (arch-aware)
set(FCITX5_ADDON_DIR "${CMAKE_INSTALL_FULL_LIBDIR}/fcitx5"
    CACHE PATH "fcitx5 addon install directory")
set(FCITX5_DATA_DIR "${CMAKE_INSTALL_FULL_DATADIR}/fcitx5"
    CACHE PATH "fcitx5 data install directory")

# ── fcitx5 header / library resolution ───────────────────────────────────────
#
# Strategy A (preferred): libfcitx5-core-dev is installed → use find_package.
# Strategy B (fallback): use a sparse checkout of the fcitx5 source tree for
#   headers and link directly against the installed versioned .so files.
#
# Strategy B is invoked automatically when find_package fails, or when
# FCITX5_USE_SRC_HEADERS=ON is set explicitly.

option(FCITX5_USE_SRC_HEADERS
    "Force use of fcitx5 source-tree headers instead of installed -dev headers"
    OFF)

if(NOT FCITX5_USE_SRC_HEADERS)
    find_package(Fcitx5Core 5.0 QUIET)
endif()

if(Fcitx5Core_FOUND AND NOT FCITX5_USE_SRC_HEADERS)
    message(STATUS "Using installed fcitx5 CMake config (${Fcitx5Core_VERSION})")
    # Fcitx5Core provides Fcitx5::Core and Fcitx5::Utils targets.
    # Nothing else to do here.
else()
    message(STATUS "fcitx5-core CMake config not found; using source-tree headers")

    # Location of the fcitx5 source lib directory.
    # Default: /tmp/fcitx5-src from sparse checkout; override for other paths.
    set(FCITX5_SRC_INCLUDE "/tmp/fcitx5-src/src/lib"
        CACHE PATH "fcitx5 src/lib directory (sparse checkout)")

    # Stub export headers (fcitxcore_export.h, etc.) generated in this repo.
    set(FCITX5_STUB_INCLUDE "${CMAKE_SOURCE_DIR}/cmake/include"
        CACHE PATH "Stub export headers for fcitx5")

    # Installed runtime libraries (from libfcitx5core7 / libfcitx5utils2).
    # These are present even without the -dev package; we just need the
    # unversioned symlinks (created via ldconfig or manual ln -s).
    set(FCITX5_CORE_LIB "/usr/lib/x86_64-linux-gnu/libFcitx5Core.so"
        CACHE FILEPATH "libFcitx5Core shared library")
    set(FCITX5_UTILS_LIB "/usr/lib/x86_64-linux-gnu/libFcitx5Utils.so"
        CACHE FILEPATH "libFcitx5Utils shared library")

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
endif()

# ── Rust FFI library ──────────────────────────────────────────────────────────

add_library(Mzkana::FFI SHARED IMPORTED)
set_target_properties(Mzkana::FFI PROPERTIES
    IMPORTED_LOCATION "${MZKANA_RUST_TARGET}/libmzkana.so"
    INTERFACE_INCLUDE_DIRECTORIES "${MZKANA_FFI_INCLUDE}"
)

# ── Addon module ──────────────────────────────────────────────────────────────

add_library(fcitx5-mzkana MODULE
    src/engine.cpp
)

target_include_directories(fcitx5-mzkana PRIVATE
    ${MZKANA_FFI_INCLUDE}
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

install(TARGETS fcitx5-mzkana
    DESTINATION "${FCITX5_ADDON_DIR}")

install(FILES data/mzkana.conf
    DESTINATION "${FCITX5_DATA_DIR}/addon")

install(FILES data/mzkana.desktop
    DESTINATION "${FCITX5_DATA_DIR}/inputmethod")

install(FILES "${MZKANA_RUST_TARGET}/libmzkana.so"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}")
