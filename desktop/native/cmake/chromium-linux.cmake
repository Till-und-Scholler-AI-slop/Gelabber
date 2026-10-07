# Toolchain for the native media core on Linux: the same clang, sysroot and
# libc++ that built the libwebrtc package. Mixing in the system libstdc++
# would break the ABI at every std:: type crossing into libwebrtc.
#
# Requires GELABBER_LIBWEBRTC_DIR (cache variable or environment).
if(NOT GELABBER_LIBWEBRTC_DIR)
  set(GELABBER_LIBWEBRTC_DIR "$ENV{GELABBER_LIBWEBRTC_DIR}")
endif()
if(NOT GELABBER_LIBWEBRTC_DIR)
  message(FATAL_ERROR "GELABBER_LIBWEBRTC_DIR is required for the Chromium toolchain")
endif()
# Try-compile projects re-read this file; forward the package location.
list(APPEND CMAKE_TRY_COMPILE_PLATFORM_VARIABLES GELABBER_LIBWEBRTC_DIR)

set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

set(_pkg "${GELABBER_LIBWEBRTC_DIR}")
set(CMAKE_C_COMPILER "${_pkg}/toolchain/bin/clang")
set(CMAKE_CXX_COMPILER "${_pkg}/toolchain/bin/clang++")
set(CMAKE_AR "${_pkg}/toolchain/bin/llvm-ar" CACHE FILEPATH "" FORCE)
set(CMAKE_SYSROOT "${_pkg}/sysroot")

set(CMAKE_C_COMPILER_TARGET x86_64-linux-gnu)
set(CMAKE_CXX_COMPILER_TARGET x86_64-linux-gnu)

# Chromium's libc++ instead of the sysroot's libstdc++. Applied by
# CMakeLists.txt with add_compile_options: a CMAKE_CXX_FLAGS_INIT here would be
# dropped whenever CMAKE_CXX_FLAGS is passed explicitly (the Rust cmake crate
# always does).
set(GELABBER_LIBCXX_FLAGS
  "-nostdinc++"
  "-isystem${_pkg}/include/buildtools/third_party/libc++"
  "-isystem${_pkg}/include/third_party/libc++/src/include"
  "-isystem${_pkg}/include/third_party/libc++abi/src/include")

# libc++/libc++abi objects come from libwebrtc.a.
set(CMAKE_SHARED_LINKER_FLAGS_INIT "-fuse-ld=lld -nostdlib++")
set(CMAKE_EXE_LINKER_FLAGS_INIT "-fuse-ld=lld -nostdlib++")

set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
