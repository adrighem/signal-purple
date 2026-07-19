# SPDX-License-Identifier: GPL-3.0-or-later
cmake_minimum_required(VERSION 3.25)

project(signal-purple VERSION 0.1.0 LANGUAGES C)

include(CTest)
include(GNUInstallDirs)
find_package(PkgConfig REQUIRED)

pkg_check_modules(PURPLE REQUIRED IMPORTED_TARGET purple>=2.14)
pkg_check_modules(GLIB REQUIRED IMPORTED_TARGET glib-2.0>=2.70 gio-2.0 gthread-2.0)
pkg_check_modules(LIBSECRET REQUIRED IMPORTED_TARGET libsecret-1>=0.20)

set(SIGNAL_PURPLE_PLUGIN_DIR "${CMAKE_INSTALL_LIBDIR}/purple-2"
    CACHE PATH "libpurple 2 protocol plugin installation directory")
set(SIGNAL_CORE_INSTALL_DIR "${SIGNAL_PURPLE_PLUGIN_DIR}/signal-purple"
    CACHE PATH "private signal-purple backend installation directory")

set(SIGNAL_CORE_MANIFEST "${CMAKE_CURRENT_SOURCE_DIR}/rust/signal-core/Cargo.toml")
set(SIGNAL_CARGO_TARGET_DIR "${CMAKE_CURRENT_SOURCE_DIR}/rust/signal-core/target"
    CACHE PATH "Cargo target directory for the Signal backend")
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(SIGNAL_CARGO_PROFILE "debug")
    set(SIGNAL_CARGO_FLAGS "")
else()
    set(SIGNAL_CARGO_PROFILE "release")
    set(SIGNAL_CARGO_FLAGS "--release")
endif()
set(SIGNAL_CORE_LIBRARY
    "${SIGNAL_CARGO_TARGET_DIR}/${SIGNAL_CARGO_PROFILE}/libsignal_core.so")

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
add_library(signal-core SHARED IMPORTED GLOBAL)
set_target_properties(signal-core PROPERTIES IMPORTED_LOCATION
                      "${SIGNAL_CORE_LIBRARY}")
add_dependencies(signal-core signal-core-build)

add_library(signal-purple MODULE
    src/contact_sync.c
    src/connection.c
    src/plugin.c
    src/util.c)
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
    PkgConfig::PURPLE PkgConfig::GLIB PkgConfig::LIBSECRET signal-core)
target_link_options(signal-purple PRIVATE "-Wl,--no-undefined")
set_target_properties(signal-purple PROPERTIES
    BUILD_RPATH "${SIGNAL_CARGO_TARGET_DIR}/${SIGNAL_CARGO_PROFILE}"
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

    add_executable(plugin-probe tests/plugin_probe.c)
    target_compile_features(plugin-probe PRIVATE c_std_11)
    target_compile_definitions(plugin-probe PRIVATE PURPLE_PLUGINS)
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
