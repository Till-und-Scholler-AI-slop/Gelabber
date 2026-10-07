cmake_minimum_required(VERSION 3.20)

# Gelabber native media core: libmediasoupclient + libwebrtc behind a C ABI
# (include/gelabber_media.h), built as one shared library.
#
# Configure with the package from scripts/build-libwebrtc-linux.sh:
#   cmake -S desktop/native -B build -G Ninja \
#     -DGELABBER_LIBWEBRTC_DIR=<package> \
#     -DCMAKE_TOOLCHAIN_FILE=desktop/native/cmake/chromium-linux.cmake
project(gelabber_media LANGUAGES C CXX)

include(FetchContent)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set(CMAKE_CXX_VISIBILITY_PRESET hidden)
set(CMAKE_VISIBILITY_INLINES_HIDDEN ON)
# libwebrtc is a release build: class layouts behind RTC_DCHECK_IS_ON must
# match, so every translation unit sees NDEBUG.
add_compile_definitions(NDEBUG)

# Set by cmake/chromium-linux.cmake; must reach libmediasoupclient and
# libsdptransform too, so it is added before FetchContent.
foreach(flag IN LISTS GELABBER_LIBCXX_FLAGS)
  add_compile_options("$<$<COMPILE_LANGUAGE:CXX>:${flag}>")
endforeach()

set(GELABBER_LIBWEBRTC_DIR "" CACHE PATH "libwebrtc package (lib/, include/)")
if(NOT GELABBER_LIBWEBRTC_DIR)
  message(FATAL_ERROR "GELABBER_LIBWEBRTC_DIR is required")
endif()

file(STRINGS "${CMAKE_CURRENT_SOURCE_DIR}/libwebrtc.env" pins REGEX "^[A-Z_]+=")
foreach(pin IN LISTS pins)
  string(REGEX REPLACE "^([A-Z_]+)=(.*)$" "\\1" key "${pin}")
  string(REGEX REPLACE "^([A-Z_]+)=(.*)$" "\\2" value "${pin}")
  set(${key} "${value}")
endforeach()

# libmediasoupclient reads these two cache variables.
set(LIBWEBRTC_INCLUDE_PATH "${GELABBER_LIBWEBRTC_DIR}/include" CACHE PATH "" FORCE)
set(LIBWEBRTC_BINARY_PATH "${GELABBER_LIBWEBRTC_DIR}/lib" CACHE PATH "" FORCE)

FetchContent_Declare(
  mediasoupclient
  GIT_REPOSITORY https://github.com/versatica/libmediasoupclient.git
  GIT_TAG ${LIBMEDIASOUPCLIENT_COMMIT}
)
FetchContent_MakeAvailable(mediasoupclient)

add_library(gelabber_media SHARED src/gelabber_media.cc)
target_include_directories(gelabber_media
  PUBLIC include
  PRIVATE ${mediasoupclient_SOURCE_DIR}/include)
target_link_libraries(gelabber_media PRIVATE mediasoupclient)

if(UNIX AND NOT APPLE)
  # WEBRTC_USE_H264 matches rtc_use_h264=true; without it the OpenH264
  # template adapters advertise no H264 formats.
  target_compile_definitions(gelabber_media PRIVATE WEBRTC_LINUX WEBRTC_USE_H264)
  target_compile_definitions(mediasoupclient PUBLIC WEBRTC_LINUX)

  # Screen capture: xdg-desktop-portal over GLib/GIO, PipeWire dlopened by
  # libwebrtc (rtc_link_pipewire=false), GBM/DRM for DMA-BUF frames. The
  # defines match libwebrtc's pipewire_config; DesktopCaptureOptions changes
  # layout with them.
  target_sources(gelabber_media PRIVATE src/screen_source_pipewire.cc)
  target_compile_definitions(gelabber_media PRIVATE WEBRTC_USE_PIPEWIRE WEBRTC_USE_GIO)
  target_include_directories(gelabber_media SYSTEM PRIVATE
    "${GELABBER_LIBWEBRTC_DIR}/include/third_party/libyuv/include"
    "${CMAKE_SYSROOT}/usr/include/glib-2.0"
    "${CMAKE_SYSROOT}/usr/lib/x86_64-linux-gnu/glib-2.0/include")
  target_link_libraries(gelabber_media PRIVATE gio-2.0 gobject-2.0 glib-2.0 gbm drm)
  target_link_options(gelabber_media PRIVATE
    # Only the gm_* C ABI leaves the library; libwebrtc, libmediasoupclient and
    # Chromium's libc++ (std::__Cr) stay internal.
    "-Wl,--version-script=${CMAKE_CURRENT_SOURCE_DIR}/gelabber_media.map"
    "-Wl,--exclude-libs,ALL"
    "-Wl,--gc-sections"
    "-Wl,-z,defs"
    "-Wl,--as-needed")
  target_link_libraries(gelabber_media PRIVATE pthread dl m rt)
endif()

install(TARGETS gelabber_media LIBRARY DESTINATION lib RUNTIME DESTINATION bin ARCHIVE DESTINATION lib)
install(FILES include/gelabber_media.h DESTINATION include)
