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

# RNNoise for the "enhanced" microphone mode: the same source commit and
# model as the web client (web/scripts/audio/build-rnnoise.sh), built as
# plain C. Neither archive has a CMake build.
FetchContent_Declare(
  rnnoise
  URL https://codeload.github.com/xiph/rnnoise/tar.gz/904a876dce1f9ab8860c0a5000ed151f9f6eef58
  URL_HASH SHA256=975488a6b6ed404b176f9e7a72b418470e50b1e7596f875eb7799f5b785e3ebf
  DOWNLOAD_NAME rnnoise-904a876d.tar.gz
)
FetchContent_Declare(
  rnnoise_model
  URL https://media.xiph.org/rnnoise/models/rnnoise_data-0b50c45.tar.gz
  URL_HASH SHA256=4ac81c5c0884ec4bd5907026aaae16209b7b76cd9d7f71af582094a2f98f4b43
)
FetchContent_MakeAvailable(rnnoise rnnoise_model)
# The model archive holds src/rnnoise_data.{c,h}; extraction may strip src/.
if(EXISTS "${rnnoise_model_SOURCE_DIR}/src/rnnoise_data.c")
  set(rnnoise_model_dir "${rnnoise_model_SOURCE_DIR}/src")
else()
  set(rnnoise_model_dir "${rnnoise_model_SOURCE_DIR}")
endif()
# v0.2's scalar vec.h includes Opus' os_support.h for OPUS_CLEAR only.
file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/rnnoise-shim/os_support.h"
  "#include <string.h>\n#define OPUS_CLEAR(dst, n) memset((dst), 0, (n)*sizeof(*(dst)))\n")
add_library(rnnoise STATIC
  ${rnnoise_SOURCE_DIR}/src/denoise.c
  ${rnnoise_SOURCE_DIR}/src/rnn.c
  ${rnnoise_SOURCE_DIR}/src/pitch.c
  ${rnnoise_SOURCE_DIR}/src/kiss_fft.c
  ${rnnoise_SOURCE_DIR}/src/celt_lpc.c
  ${rnnoise_SOURCE_DIR}/src/nnet.c
  ${rnnoise_SOURCE_DIR}/src/nnet_default.c
  ${rnnoise_SOURCE_DIR}/src/parse_lpcnet_weights.c
  ${rnnoise_model_dir}/rnnoise_data.c
  ${rnnoise_SOURCE_DIR}/src/rnnoise_tables.c)
target_include_directories(rnnoise
  PUBLIC ${rnnoise_SOURCE_DIR}/include
  PRIVATE ${rnnoise_model_dir} ${rnnoise_SOURCE_DIR}/src "${CMAKE_CURRENT_BINARY_DIR}/rnnoise-shim")
target_compile_options(rnnoise PRIVATE -w)

add_library(gelabber_media SHARED src/gelabber_media.cc src/capture_dsp.cc src/camera_source.cc)
target_include_directories(gelabber_media
  PUBLIC include
  PRIVATE ${mediasoupclient_SOURCE_DIR}/include)
target_link_libraries(gelabber_media PRIVATE mediasoupclient rnnoise)

if(UNIX AND NOT APPLE)
  # WEBRTC_USE_H264 matches rtc_use_h264=true; without it the OpenH264
  # template adapters advertise no H264 formats.
  target_compile_definitions(gelabber_media PRIVATE WEBRTC_LINUX WEBRTC_USE_H264)
  target_compile_definitions(mediasoupclient PUBLIC WEBRTC_LINUX)

  # Screen capture: xdg-desktop-portal over GLib/GIO, PipeWire dlopened by
  # libwebrtc (rtc_link_pipewire=false), GBM/DRM for DMA-BUF frames. The
  # defines match libwebrtc's pipewire_config; DesktopCaptureOptions changes
  # layout with them. Hardware H264 loads the system GStreamer with dlopen.
  target_sources(gelabber_media PRIVATE src/screen_source_pipewire.cc src/gst_h264_encoder.cc
    src/app_audio_pipewire.cc)
  target_compile_definitions(gelabber_media PRIVATE WEBRTC_USE_PIPEWIRE WEBRTC_USE_GIO)
  target_include_directories(gelabber_media SYSTEM PRIVATE
    "${GELABBER_LIBWEBRTC_DIR}/include/third_party/libyuv/include"
    "${CMAKE_SYSROOT}/usr/include/glib-2.0"
    # Application sound: libpipewire is dlopened, only its headers are used.
    "${CMAKE_SYSROOT}/usr/include/pipewire-0.3"
    "${CMAKE_SYSROOT}/usr/include/spa-0.2"
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
