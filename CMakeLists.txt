# SPDX-License-Identifier: GPL-3.0-or-later
cmake_minimum_required(VERSION 3.25)

file(STRINGS "${CMAKE_CURRENT_SOURCE_DIR}/version.txt"
     SIGNAL_PURPLE_VERSION LIMIT_COUNT 1)
if(NOT SIGNAL_PURPLE_VERSION MATCHES "^[0-9]+\\.[0-9]+\\.[0-9]+$")
    message(FATAL_ERROR "version.txt must contain one semantic version")
endif()

project(signal-purple VERSION "${SIGNAL_PURPLE_VERSION}" LANGUAGES C)

include(CTest)
include(GNUInstallDirs)
find_package(PkgConfig REQUIRED)

pkg_check_modules(PURPLE REQUIRED IMPORTED_TARGET purple>=2.14)
pkg_check_modules(GLIB REQUIRED IMPORTED_TARGET
    glib-2.0>=2.70 gio-2.0 gmodule-2.0 gthread-2.0)
pkg_check_modules(GDK_PIXBUF REQUIRED IMPORTED_TARGET gdk-pixbuf-2.0>=2.42)
pkg_check_modules(LIBSECRET REQUIRED IMPORTED_TARGET libsecret-1>=0.20)

set(SIGNAL_PURPLE_PLUGIN_DIR "${CMAKE_INSTALL_LIBDIR}/purple-2"
    CACHE PATH "libpurple 2 protocol plugin installation directory")
set(SIGNAL_CORE_INSTALL_DIR "${SIGNAL_PURPLE_PLUGIN_DIR}/signal-purple"
    CACHE PATH "private signal-purple backend installation directory")

set(SIGNAL_CORE_MANIFEST "${CMAKE_CURRENT_SOURCE_DIR}/rust/signal-core/Cargo.toml")
file(STRINGS "${SIGNAL_CORE_MANIFEST}" SIGNAL_CORE_VERSION_LINE
     REGEX "^version = " LIMIT_COUNT 1)
if(NOT SIGNAL_CORE_VERSION_LINE MATCHES
   "version = \"${PROJECT_VERSION}\"")
    message(FATAL_ERROR "Rust and product versions do not match")
endif()

file(STRINGS "${CMAKE_CURRENT_SOURCE_DIR}/CITATION.cff"
     SIGNAL_CITATION_VERSION_LINE REGEX "^version: " LIMIT_COUNT 1)
if(NOT SIGNAL_CITATION_VERSION_LINE MATCHES
   "version: ${PROJECT_VERSION}")
    message(FATAL_ERROR "Citation and product versions do not match")
endif()

file(READ "${CMAKE_CURRENT_SOURCE_DIR}/rust/signal-core/Cargo.lock"
     SIGNAL_CORE_LOCK)
string(FIND "${SIGNAL_CORE_LOCK}"
       "name = \"signal-core\"\nversion = \"${PROJECT_VERSION}\""
       SIGNAL_CORE_LOCK_VERSION_POSITION)
if(SIGNAL_CORE_LOCK_VERSION_POSITION EQUAL -1)
    message(FATAL_ERROR "Cargo lock and product versions do not match")
endif()
set(SIGNAL_CARGO_TARGET_DIR "${CMAKE_CURRENT_SOURCE_DIR}/rust/signal-core/target"
    CACHE PATH "Cargo target directory for the Signal backend")
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(SIGNAL_CARGO_PROFILE "debug")
    set(SIGNAL_CARGO_FLAGS)
else()
    set(SIGNAL_CARGO_PROFILE "release")
    set(SIGNAL_CARGO_FLAGS --release)
endif()
if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/vendor")
    list(APPEND SIGNAL_CARGO_FLAGS --offline)
endif()
set(SIGNAL_CORE_LIBRARY
    "${SIGNAL_CARGO_TARGET_DIR}/${SIGNAL_CARGO_PROFILE}/libsignal_core.so")
set(SIGNAL_CORE_BUILD_RUNTIME_DIR
    "${CMAKE_CURRENT_BINARY_DIR}/signal-purple")
set(SIGNAL_CORE_BUILD_RUNTIME_LIBRARY
    "${SIGNAL_CORE_BUILD_RUNTIME_DIR}/libsignal_core.so")

find_program(CARGO_EXECUTABLE cargo REQUIRED)
add_custom_command(
    OUTPUT "${SIGNAL_CORE_LIBRARY}"
    COMMAND "${CMAKE_COMMAND}" -E env
            "CARGO_TARGET_DIR=${SIGNAL_CARGO_TARGET_DIR}"
            "${CARGO_EXECUTABLE}" build --locked ${SIGNAL_CARGO_FLAGS}
            --manifest-path "${SIGNAL_CORE_MANIFEST}"
    DEPENDS
        "${SIGNAL_CORE_MANIFEST}"
        "${CMAKE_CURRENT_SOURCE_DIR}/rust/signal-core/Cargo.lock"
        "${CMAKE_CURRENT_SOURCE_DIR}/rust/signal-core/src/lib.rs"
        "${CMAKE_CURRENT_SOURCE_DIR}/rust/signal-core/src/backend.rs"
        "${CMAKE_CURRENT_SOURCE_DIR}/rust/signal-core/src/event.rs"
        "${CMAKE_CURRENT_SOURCE_DIR}/rust/signal-core/src/ffi.rs"
        "${CMAKE_CURRENT_SOURCE_DIR}/rust-toolchain.toml"
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}"
    COMMENT "Building the pinned Presage backend"
    VERBATIM)
add_custom_target(signal-core-build ALL DEPENDS "${SIGNAL_CORE_LIBRARY}")
add_custom_command(
    OUTPUT "${SIGNAL_CORE_BUILD_RUNTIME_LIBRARY}"
    COMMAND "${CMAKE_COMMAND}" -E make_directory
            "${SIGNAL_CORE_BUILD_RUNTIME_DIR}"
    COMMAND "${CMAKE_COMMAND}" -E create_symlink
            "${SIGNAL_CORE_LIBRARY}"
            "${SIGNAL_CORE_BUILD_RUNTIME_LIBRARY}"
    DEPENDS "${SIGNAL_CORE_LIBRARY}"
    VERBATIM)
add_custom_target(signal-core-runtime ALL
                  DEPENDS "${SIGNAL_CORE_BUILD_RUNTIME_LIBRARY}")
add_library(signal-core SHARED IMPORTED GLOBAL)
set_target_properties(signal-core PROPERTIES IMPORTED_LOCATION
                      "${SIGNAL_CORE_LIBRARY}"
                      IMPORTED_NO_SONAME TRUE)
add_dependencies(signal-core signal-core-build)

add_library(signal-purple MODULE
    src/blist_sync.c
    src/contact_sync.c
    src/connection.c
    src/group_sync.c
    src/inline_image.c
    src/plugin.c
    src/util.c)
add_dependencies(signal-purple signal-core-runtime)
target_compile_features(signal-purple PRIVATE c_std_11)
target_compile_definitions(signal-purple PRIVATE
    PURPLE_PLUGINS
    SIGNAL_PURPLE_VERSION="${PROJECT_VERSION}")
target_compile_options(signal-purple PRIVATE
    -Wall -Wextra -Wpedantic -Wconversion -Wformat=2 -Wshadow)
target_include_directories(signal-purple PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/include"
    "${CMAKE_CURRENT_SOURCE_DIR}/src")
target_link_libraries(signal-purple PRIVATE
    PkgConfig::PURPLE PkgConfig::GLIB PkgConfig::GDK_PIXBUF
    PkgConfig::LIBSECRET signal-core)
target_link_options(signal-purple PRIVATE "-Wl,--no-undefined")
set_target_properties(signal-purple PROPERTIES
    BUILD_RPATH "$ORIGIN/signal-purple"
    BUILD_WITH_INSTALL_RPATH TRUE
    INSTALL_RPATH "$ORIGIN/signal-purple")

install(TARGETS signal-purple LIBRARY DESTINATION "${SIGNAL_PURPLE_PLUGIN_DIR}")
install(FILES "${SIGNAL_CORE_LIBRARY}"
        DESTINATION "${SIGNAL_CORE_INSTALL_DIR}")
install(FILES data/io.github.adrighem.signal-purple.metainfo.xml
        DESTINATION "${CMAKE_INSTALL_DATAROOTDIR}/metainfo")
foreach(size IN ITEMS 16 22 48)
    install(FILES "${CMAKE_CURRENT_SOURCE_DIR}/data/icons/${size}/signal-purple.png"
            DESTINATION "${CMAKE_INSTALL_DATAROOTDIR}/pixmaps/pidgin/protocols/${size}")
endforeach()

if(BUILD_TESTING)
    add_executable(test-util tests/test_util.c src/util.c)
    target_compile_features(test-util PRIVATE c_std_11)
    target_compile_options(test-util PRIVATE -Wall -Wextra -Wpedantic)
    target_include_directories(test-util PRIVATE
        "${CMAKE_CURRENT_SOURCE_DIR}/include"
        "${CMAKE_CURRENT_SOURCE_DIR}/src")
    target_link_libraries(test-util PRIVATE
        PkgConfig::PURPLE PkgConfig::GLIB PkgConfig::LIBSECRET)
    add_test(NAME util COMMAND test-util)

    add_executable(test-contact-sync
        tests/test_contact_sync.c
        src/contact_sync.c)
    target_compile_features(test-contact-sync PRIVATE c_std_11)
    target_compile_options(test-contact-sync PRIVATE
        -Wall -Wextra -Wpedantic -Wconversion -Wformat=2 -Wshadow)
    target_include_directories(test-contact-sync PRIVATE
        "${CMAKE_CURRENT_SOURCE_DIR}/src")
    target_link_libraries(test-contact-sync PRIVATE PkgConfig::GLIB)
    add_test(NAME contact-sync COMMAND test-contact-sync)

    add_executable(test-inline-image
        tests/test_inline_image.c
        src/inline_image.c)
    target_compile_features(test-inline-image PRIVATE c_std_11)
    target_compile_options(test-inline-image PRIVATE
        -Wall -Wextra -Wpedantic -Wconversion -Wformat=2 -Wshadow)
    target_include_directories(test-inline-image PRIVATE
        "${CMAKE_CURRENT_SOURCE_DIR}/src")
    target_link_libraries(test-inline-image PRIVATE
        PkgConfig::PURPLE PkgConfig::GLIB PkgConfig::GDK_PIXBUF)
    add_test(NAME inline-image COMMAND test-inline-image)

    add_executable(plugin-probe
        tests/plugin_probe.c
        src/blist_sync.c
        src/contact_sync.c
        src/group_sync.c)
    target_compile_features(plugin-probe PRIVATE c_std_11)
    target_compile_definitions(plugin-probe PRIVATE
        PURPLE_PLUGINS
        EXPECTED_PLUGIN_VERSION="${PROJECT_VERSION}")
    target_compile_options(plugin-probe PRIVATE -Wall -Wextra -Wpedantic)
    target_include_directories(plugin-probe PRIVATE
        "${CMAKE_CURRENT_SOURCE_DIR}/include"
        "${CMAKE_CURRENT_SOURCE_DIR}/src")
    target_link_libraries(plugin-probe PRIVATE PkgConfig::PURPLE PkgConfig::GLIB)
    add_dependencies(plugin-probe signal-purple)
    add_test(NAME plugin-probe
             COMMAND plugin-probe "$<TARGET_FILE:signal-purple>")
    set_tests_properties(plugin-probe PROPERTIES
        ENVIRONMENT "G_DEBUG=fatal-warnings")
endif()
