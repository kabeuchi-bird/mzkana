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

# Auto-detect the fcitx5 addon directory from the installed libFcitx5Core location.
# This avoids requiring -DCMAKE_INSTALL_PREFIX=/usr when the system fcitx5 is under /usr.
find_library(_fcitx5_core_lib NAMES Fcitx5Core)
if(_fcitx5_core_lib)
    get_filename_component(_fcitx5_lib_dir "${_fcitx5_core_lib}" DIRECTORY)
    set(_fcitx5_addon_default "${_fcitx5_lib_dir}/fcitx5")
    # Derive data dir: walk up from lib dir to prefix, then append share/fcitx5.
    # e.g. /usr/lib/x86_64-linux-gnu → /usr → /usr/share/fcitx5
    get_filename_component(_fcitx5_prefix "${_fcitx5_lib_dir}" DIRECTORY)
    if("${_fcitx5_prefix}" MATCHES "/lib$")
        get_filename_component(_fcitx5_prefix "${_fcitx5_prefix}" DIRECTORY)
    endif()
    set(_fcitx5_data_default "${_fcitx5_prefix}/share/fcitx5")
else()
    set(_fcitx5_addon_default "${CMAKE_INSTALL_FULL_LIBDIR}/fcitx5")
    set(_fcitx5_data_default "${CMAKE_INSTALL_FULL_DATADIR}/fcitx5")
endif()

# Install destinations (override with -DFCITX5_ADDON_DIR=... if needed)
set(FCITX5_ADDON_DIR "${_fcitx5_addon_default}"
    CACHE PATH "fcitx5 addon install directory")
set(FCITX5_DATA_DIR "${_fcitx5_data_default}"
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
    # The config API (fcitx-config/*, used by the settings GUI, §13.5) lives in
    # the separate Fcitx5Config package.
    find_package(Fcitx5Config 5.0 REQUIRED)
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
    set(FCITX5_CONFIG_LIB "/usr/lib/x86_64-linux-gnu/libFcitx5Config.so"
        CACHE FILEPATH "libFcitx5Config shared library")

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

    # fcitx-config/* headers (settings GUI, §13.5).
    add_library(Fcitx5::Config SHARED IMPORTED)
    set_target_properties(Fcitx5::Config PROPERTIES
        IMPORTED_LOCATION "${FCITX5_CONFIG_LIB}"
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
    Fcitx5::Config
)

# Fcitx5 addon .so must be named without the "lib" prefix.
# INSTALL_RPATH tells the dynamic linker where libmzkana.so lives after
# installation so that the NEEDED entry (set by the SONAME in build.rs)
# can be resolved without requiring ldconfig or LD_LIBRARY_PATH.
set_target_properties(fcitx5-mzkana PROPERTIES
    PREFIX ""
    INSTALL_RPATH "${CMAKE_INSTALL_FULL_LIBDIR}"
    BUILD_WITH_INSTALL_RPATH TRUE
)

# ── Install ───────────────────────────────────────────────────────────────────

install(TARGETS fcitx5-mzkana
    DESTINATION "${FCITX5_ADDON_DIR}")

install(FILES data/mzkana.conf
    DESTINATION "${FCITX5_DATA_DIR}/addon")

install(FILES data/mzkana-im.conf
    DESTINATION "${FCITX5_DATA_DIR}/inputmethod"
    RENAME mzkana.conf)

install(FILES "${MZKANA_RUST_TARGET}/libmzkana.so"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}")
